//!
//! Module implementing [`Resolver`] client for obtaining public Kaspa wRPC endpoints.
//!

use std::sync::OnceLock;

use crate::error::Error;
use crate::imports::*;
use crate::node::NodeDescriptor;
pub use futures::future::join_all;
use rand::seq::SliceRandom;
use rand::thread_rng;
use workflow_core::runtime;
use workflow_http::get_json;

const CURRENT_VERSION: usize = 2;
const RESOLVER_CONFIG: &str = include_str!("../Resolvers.toml");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolverRecord {
    pub address: String,
    pub enable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolverGroup {
    pub template: String,
    pub nodes: Vec<String>,
    pub enable: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResolverConfig {
    // Both default to empty. A config listing no endpoints is a legitimate
    // state — it is Marigold's, having declined to inherit Kaspa's public node
    // list — and without these the file fails to parse, which `Inner::new`
    // turns into an `expect` and a panic before the CLI can even draw a
    // prompt. A network with no public nodes must not be an unstartable one.
    #[serde(rename = "group", default)]
    groups: Vec<ResolverGroup>,
    #[serde(rename = "resolver", default)]
    resolvers: Vec<ResolverRecord>,
}

fn try_parse_resolvers(toml: &str) -> Result<Vec<Arc<String>>> {
    let config = toml::from_str::<ResolverConfig>(toml)?;

    let mut resolvers = config
        .resolvers
        .into_iter()
        .filter_map(|resolver| resolver.enable.unwrap_or(true).then_some(resolver.address))
        .collect::<Vec<_>>();

    let groups = config.groups.into_iter().filter(|group| group.enable.unwrap_or(true)).collect::<Vec<_>>();

    for group in groups {
        let ResolverGroup { template, nodes, .. } = group;
        for node in nodes {
            resolvers.push(template.replace('*', &node));
        }
    }

    Ok(resolvers.into_iter().map(Arc::new).collect::<Vec<_>>())
}

#[derive(Debug)]
struct Inner {
    pub urls: Vec<Arc<String>>,
    pub tls: bool,
    public: bool,
}

impl Inner {
    pub fn new(urls: Option<Vec<Arc<String>>>, tls: bool) -> Self {
        if urls.as_ref().is_some_and(|urls| urls.is_empty()) {
            panic!("Resolver: Empty URL list supplied to the constructor.");
        }

        let mut public = false;
        let urls = urls.unwrap_or_else(|| {
            public = true;
            try_parse_resolvers(RESOLVER_CONFIG).expect("TOML: Unable to parse RPC Resolver list")
        });

        Self { urls, tls, public }
    }
}

///
/// # Resolver - a client for obtaining public Kaspa wRPC endpoints.
///
/// This client operates against [Kaspa Resolver](https://github.com/aspectron/kaspa-resolver) service
/// that provides load-balancing and failover capabilities for Kaspa wRPC endpoints. The default
/// configuration allows access to public Kaspa nodes, while custom configurations can be supplied
/// if you are running your own custom Kaspa node cluster.
///
#[derive(Debug, Clone)]
pub struct Resolver {
    inner: Arc<Inner>,
}

impl Default for Resolver {
    fn default() -> Self {
        Self { inner: Arc::new(Inner::new(None, false)) }
    }
}

impl Resolver {
    /// Create a new [`Resolver`] client with the specified list of resolver URLs and an optional `tls` flag.
    /// The `tls` flag can be used to enforce secure connection to the node.
    pub fn new(urls: Option<Vec<Arc<String>>>, tls: bool) -> Self {
        Self { inner: Arc::new(Inner::new(urls, tls)) }
    }

    /// Obtain a list of URLs in the resolver client. (This function
    /// returns `None` if the resolver is configured to use public
    /// node endpoints.)
    pub fn urls(&self) -> Option<Vec<Arc<String>>> {
        if self.inner.public { None } else { Some(self.inner.urls.clone()) }
    }

    /// Whether any public endpoint is configured at all.
    ///
    /// Marigold ships none: the fork emptied `Resolvers.toml` rather than
    /// inherit Kaspa's public node list, which would have handed a Marigold
    /// wallet a Kaspa node. Callers check this so they can say so plainly
    /// instead of reporting a connection failure against an empty list.
    pub fn is_configured(&self) -> bool {
        !self.inner.urls.is_empty()
    }

    /// Obtain the `tls` flag in the resolver client.
    pub fn tls(&self) -> bool {
        self.inner.tls
    }

    fn tls_as_str(&self) -> &'static str {
        if self.inner.tls { "tls" } else { "any" }
    }

    fn make_url(&self, url: &str, encoding: Encoding, network_id: NetworkId) -> String {
        static TLS: OnceLock<&'static str> = OnceLock::new();

        let tls = *TLS.get_or_init(|| {
            if runtime::is_web() {
                let tls = js_sys::Reflect::get(&js_sys::global(), &"location".into())
                    .and_then(|location| js_sys::Reflect::get(&location, &"protocol".into()))
                    .ok()
                    .and_then(|protocol| protocol.as_string())
                    .map(|protocol| protocol.starts_with("https"))
                    .unwrap_or(false);
                if tls { "tls" } else { self.tls_as_str() }
            } else {
                self.tls_as_str()
            }
        });

        format!("{url}/v{CURRENT_VERSION}/kaspa/{network_id}/{tls}/wrpc/{encoding}")
    }

    // query a single resolver service
    async fn fetch_node_info(&self, url: &str, encoding: Encoding, network_id: NetworkId) -> Result<NodeDescriptor> {
        let url = self.make_url(url, encoding, network_id);
        let node =
            get_json::<NodeDescriptor>(&url).await.map_err(|error| Error::custom(format!("Unable to connect to {url}: {error}")))?;
        Ok(node)
    }

    // query multiple resolver services in random order
    async fn fetch(&self, encoding: Encoding, network_id: NetworkId) -> Result<NodeDescriptor> {
        if self.inner.urls.is_empty() {
            return Err(Error::Custom(
                "no public nodes are configured for this network — connect to a node by address instead".to_string(),
            ));
        }
        let mut urls = self.inner.urls.clone();
        urls.shuffle(&mut thread_rng());

        let mut errors = Vec::default();
        for url in urls {
            match self.fetch_node_info(&url, encoding, network_id).await {
                Ok(node) => return Ok(node),
                Err(error) => errors.push(error),
            }
        }
        Err(Error::Custom(format!("Failed to connect: {:?}", errors)))
    }

    /// Obtain a Kaspa p2p [`NodeDescriptor`] from the resolver based on the supplied [`Encoding`] and [`NetworkId`].
    pub async fn get_node(&self, encoding: Encoding, network_id: NetworkId) -> Result<NodeDescriptor> {
        self.fetch(encoding, network_id).await
    }

    /// Returns a Kaspa wRPC URL from the resolver based on the supplied [`Encoding`] and [`NetworkId`].
    pub async fn get_url(&self, encoding: Encoding, network_id: NetworkId) -> Result<String> {
        let nodes = self.fetch(encoding, network_id).await?;
        Ok(nodes.url.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolver_config_1() {
        let toml = r#"
            [[group]]
            enable = true
            template = "https://*.example.org"
            nodes = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"]

            [[group]]
            enable = true
            template = "https://*.example.com"
            nodes = ["iota", "kappa", "lambda", "mu", "nu", "xi", "omicron", "pi"]

            [[resolver]]
            enable = true
            address = "http://127.0.0.1:8888"
        "#;

        let urls = try_parse_resolvers(toml).expect("TOML: Unable to parse RPC Resolver list");
        // println!("{:#?}", urls);
        assert_eq!(urls.len(), 17);
    }

    #[test]
    fn test_resolver_config_2() {
        let _urls = try_parse_resolvers(RESOLVER_CONFIG).expect("TOML: Unable to parse RPC Resolver list");
        // println!("{:#?}", urls);
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    /// The shipped config must parse. It is `include_str!`d and `Inner::new`
    /// expects it, so a config that does not parse is a panic before the
    /// process can print anything — which is exactly what an all-comments
    /// Resolvers.toml did when Marigold emptied it (2026-09-07).
    #[test]
    fn the_built_in_config_parses() {
        let urls = try_parse_resolvers(RESOLVER_CONFIG).expect("the shipped Resolvers.toml must parse");
        // Marigold ships none. If this ever becomes non-empty, it must be
        // because a Marigold node was deliberately exposed — never because
        // Kaspa's list came back through an upstream merge.
        for url in &urls {
            assert!(!url.contains("kaspa"), "a Kaspa endpoint has reappeared in Resolvers.toml: {url}");
        }
    }

    /// A resolver with nothing configured must report itself as such, so a
    /// caller can say "this network has no public nodes" rather than surface a
    /// failed connection against an empty list.
    #[test]
    fn an_empty_resolver_reports_itself_unconfigured() {
        assert!(!Resolver::default().is_configured());
    }
}

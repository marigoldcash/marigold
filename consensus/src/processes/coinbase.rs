use kaspa_consensus_core::{
    BlockHashMap, BlockHashSet,
    coinbase::*,
    config::params::{ForkActivation, ForkedParam},
    errors::coinbase::{CoinbaseError, CoinbaseResult},
    subnets,
    tx::{ScriptPublicKey, ScriptVec, Transaction, TransactionOutput},
};
use std::convert::TryInto;

use crate::{constants, model::stores::ghostdag::GhostdagData};

const LENGTH_OF_BLUE_SCORE: usize = size_of::<u64>();
const LENGTH_OF_SUBSIDY: usize = size_of::<u64>();
const LENGTH_OF_SCRIPT_PUB_KEY_VERSION: usize = size_of::<u16>();
const LENGTH_OF_SCRIPT_PUB_KEY_LENGTH: usize = size_of::<u8>();

const MIN_PAYLOAD_LENGTH: usize =
    LENGTH_OF_BLUE_SCORE + LENGTH_OF_SUBSIDY + LENGTH_OF_SCRIPT_PUB_KEY_VERSION + LENGTH_OF_SCRIPT_PUB_KEY_LENGTH;

// We define a year as 365.25 days and a month as 365.25 / 12 = 30.4375
// SECONDS_PER_MONTH = 30.4375 * 24 * 60 * 60
const SECONDS_PER_MONTH: u64 = 2629800;

pub const SUBSIDY_BY_MONTH_TABLE_SIZE: usize = 1016;
pub type SubsidyByMonthTable = [u64; SUBSIDY_BY_MONTH_TABLE_SIZE];

#[derive(Clone)]
pub struct CoinbaseManager {
    coinbase_payload_script_public_key_max_len: u8,
    max_coinbase_payload_len: usize,
    deflationary_phase_daa_score: u64,
    pre_deflationary_phase_base_subsidy: u64,
    bps_history: ForkedParam<u64>,
    toccata_activation: ForkActivation,

    /// Precomputed subsidy by month tables (for before and after the Crescendo hardfork)
    subsidy_by_month_table_before: SubsidyByMonthTable,
    subsidy_by_month_table_after: SubsidyByMonthTable,

    /// The crescendo activation DAA score where BPS increased from 1 to 10.
    /// This score is required here long-term (and not only for the actual forking), in
    /// order to correctly determine the subsidy month from the live DAA score of the network   
    crescendo_activation_daa_score: u64,
}

/// Struct used to streamline payload parsing
struct PayloadParser<'a> {
    remaining: &'a [u8], // The unparsed remainder
}

impl<'a> PayloadParser<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { remaining: data }
    }

    /// Returns a slice with the first `n` bytes of `remaining`, while setting `remaining` to the remaining part
    fn take(&mut self, n: usize) -> &[u8] {
        let (segment, remaining) = self.remaining.split_at(n);
        self.remaining = remaining;
        segment
    }
}

impl CoinbaseManager {
    pub fn new(
        coinbase_payload_script_public_key_max_len: u8,
        max_coinbase_payload_len: usize,
        deflationary_phase_daa_score: u64,
        pre_deflationary_phase_base_subsidy: u64,
        bps_history: ForkedParam<u64>,
        toccata_activation: ForkActivation,
    ) -> Self {
        // Precomputed subsidy by month table for the actual block per second rate
        // Here values are rounded up so that we keep the same number of rewarding months as in the original 1 BPS table.
        // In a 10 BPS network, the induced increase in total rewards is 51 KAS (see tests::calc_high_bps_total_rewards_delta())
        let subsidy_by_month_table_before: SubsidyByMonthTable =
            core::array::from_fn(|i| SUBSIDY_BY_MONTH_TABLE[i].div_ceil(bps_history.before()));
        let subsidy_by_month_table_after: SubsidyByMonthTable =
            core::array::from_fn(|i| SUBSIDY_BY_MONTH_TABLE[i].div_ceil(bps_history.after()));
        Self {
            coinbase_payload_script_public_key_max_len,
            max_coinbase_payload_len,
            deflationary_phase_daa_score,
            pre_deflationary_phase_base_subsidy,
            bps_history,
            toccata_activation,
            subsidy_by_month_table_before,
            subsidy_by_month_table_after,
            crescendo_activation_daa_score: bps_history.activation().daa_score(),
        }
    }

    #[cfg(test)]
    #[inline]
    pub fn bps(&self) -> ForkedParam<u64> {
        self.bps_history
    }

    pub fn expected_coinbase_transaction<T: AsRef<[u8]>>(
        &self,
        daa_score: u64,
        miner_data: MinerData<T>,
        ghostdag_data: &GhostdagData,
        mergeset_rewards: &BlockHashMap<BlockRewardData>,
        mergeset_non_daa: &BlockHashSet,
    ) -> CoinbaseResult<CoinbaseTransactionTemplate> {
        let mut outputs = Vec::with_capacity(ghostdag_data.mergeset_blues.len() + 1); // + 1 for possible red reward

        // Add an output for each mergeset blue block (∩ DAA window), paying to the script reported by the block.
        // Note that combinatorically it is nearly impossible for a blue block to be non-DAA
        for blue in ghostdag_data.mergeset_blues.iter().filter(|h| !mergeset_non_daa.contains(h)) {
            let reward_data = mergeset_rewards.get(blue).unwrap();
            if reward_data.subsidy + reward_data.total_fees > 0 {
                outputs
                    .push(TransactionOutput::new(reward_data.subsidy + reward_data.total_fees, reward_data.script_public_key.clone()));
            }
        }

        // Collect all rewards from mergeset reds ∩ DAA window and create a
        // single output rewarding all to the current block (the "merging" block)
        let mut red_reward = 0u64;

        for red in ghostdag_data.mergeset_reds.iter() {
            let reward_data = mergeset_rewards.get(red).unwrap();
            if mergeset_non_daa.contains(red) {
                red_reward += reward_data.total_fees;
            } else {
                red_reward += reward_data.subsidy + reward_data.total_fees;
            }
        }

        if red_reward > 0 {
            outputs.push(TransactionOutput::new(red_reward, miner_data.script_public_key.clone()));
        }

        // Build the current block's payload
        let subsidy = self.calc_block_subsidy(daa_score);
        let payload = self.serialize_coinbase_payload(&CoinbaseData { blue_score: ghostdag_data.blue_score, subsidy, miner_data })?;

        let tx_version =
            if self.toccata_activation.is_active(daa_score) { constants::TX_VERSION_TOCCATA } else { constants::TX_VERSION };

        Ok(CoinbaseTransactionTemplate {
            tx: Transaction::new(tx_version, vec![], outputs, 0, subnets::SUBNETWORK_ID_COINBASE, 0, payload),
            has_red_reward: red_reward > 0,
        })
    }

    pub fn serialize_coinbase_payload<T: AsRef<[u8]>>(&self, data: &CoinbaseData<T>) -> CoinbaseResult<Vec<u8>> {
        let script_pub_key_len = data.miner_data.script_public_key.script().len();
        if script_pub_key_len > self.coinbase_payload_script_public_key_max_len as usize {
            return Err(CoinbaseError::PayloadScriptPublicKeyLenAboveMax(
                script_pub_key_len,
                self.coinbase_payload_script_public_key_max_len,
            ));
        }
        let payload: Vec<u8> = data.blue_score.to_le_bytes().iter().copied()                    // Blue score                   (u64)
            .chain(data.subsidy.to_le_bytes().iter().copied())                                  // Subsidy                      (u64)
            .chain(data.miner_data.script_public_key.version().to_le_bytes().iter().copied())   // Script public key version    (u16)
            .chain((script_pub_key_len as u8).to_le_bytes().iter().copied())                    // Script public key length     (u8)
            .chain(data.miner_data.script_public_key.script().iter().copied())                  // Script public key            
            .chain(data.miner_data.extra_data.as_ref().iter().copied())                         // Extra data
            .collect();

        Ok(payload)
    }

    pub fn modify_coinbase_payload<T: AsRef<[u8]>>(&self, mut payload: Vec<u8>, miner_data: &MinerData<T>) -> CoinbaseResult<Vec<u8>> {
        let script_pub_key_len = miner_data.script_public_key.script().len();
        if script_pub_key_len > self.coinbase_payload_script_public_key_max_len as usize {
            return Err(CoinbaseError::PayloadScriptPublicKeyLenAboveMax(
                script_pub_key_len,
                self.coinbase_payload_script_public_key_max_len,
            ));
        }

        // Keep only blue score and subsidy. Note that truncate does not modify capacity, so
        // the usual case where the payloads are the same size will not trigger a reallocation
        payload.truncate(LENGTH_OF_BLUE_SCORE + LENGTH_OF_SUBSIDY);
        payload.extend(
            miner_data.script_public_key.version().to_le_bytes().iter().copied() // Script public key version (u16)
                .chain((script_pub_key_len as u8).to_le_bytes().iter().copied()) // Script public key length  (u8)
                .chain(miner_data.script_public_key.script().iter().copied())    // Script public key
                .chain(miner_data.extra_data.as_ref().iter().copied()), // Extra data
        );

        Ok(payload)
    }

    pub fn deserialize_coinbase_payload<'a>(&self, payload: &'a [u8]) -> CoinbaseResult<CoinbaseData<&'a [u8]>> {
        if payload.len() < MIN_PAYLOAD_LENGTH {
            return Err(CoinbaseError::PayloadLenBelowMin(payload.len(), MIN_PAYLOAD_LENGTH));
        }

        if payload.len() > self.max_coinbase_payload_len {
            return Err(CoinbaseError::PayloadLenAboveMax(payload.len(), self.max_coinbase_payload_len));
        }

        let mut parser = PayloadParser::new(payload);

        let blue_score = u64::from_le_bytes(parser.take(LENGTH_OF_BLUE_SCORE).try_into().unwrap());
        let subsidy = u64::from_le_bytes(parser.take(LENGTH_OF_SUBSIDY).try_into().unwrap());
        let script_pub_key_version = u16::from_le_bytes(parser.take(LENGTH_OF_SCRIPT_PUB_KEY_VERSION).try_into().unwrap());
        let script_pub_key_len = u8::from_le_bytes(parser.take(LENGTH_OF_SCRIPT_PUB_KEY_LENGTH).try_into().unwrap());

        if script_pub_key_len > self.coinbase_payload_script_public_key_max_len {
            return Err(CoinbaseError::PayloadScriptPublicKeyLenAboveMax(
                script_pub_key_len as usize,
                self.coinbase_payload_script_public_key_max_len,
            ));
        }

        if parser.remaining.len() < script_pub_key_len as usize {
            return Err(CoinbaseError::PayloadCantContainScriptPublicKey(
                payload.len(),
                MIN_PAYLOAD_LENGTH + script_pub_key_len as usize,
            ));
        }

        let script_public_key =
            ScriptPublicKey::new(script_pub_key_version, ScriptVec::from_slice(parser.take(script_pub_key_len as usize)));
        let extra_data = parser.remaining;

        Ok(CoinbaseData { blue_score, subsidy, miner_data: MinerData { script_public_key, extra_data } })
    }

    pub fn calc_block_subsidy(&self, daa_score: u64) -> u64 {
        if daa_score < self.deflationary_phase_daa_score {
            return self.pre_deflationary_phase_base_subsidy;
        }

        let subsidy_month = self.subsidy_month(daa_score) as usize;
        let subsidy_table = if self.bps_history.activation().is_active(daa_score) {
            &self.subsidy_by_month_table_after
        } else {
            &self.subsidy_by_month_table_before
        };
        subsidy_table[subsidy_month.min(subsidy_table.len() - 1)]
    }

    /// Get the subsidy month as function of the current DAA score.
    ///
    /// Note that this function is called only if daa_score >= self.deflationary_phase_daa_score
    fn subsidy_month(&self, daa_score: u64) -> u64 {
        let seconds_since_deflationary_phase_started = if self.crescendo_activation_daa_score < self.deflationary_phase_daa_score {
            // crescendo_activation < deflationary_phase <= daa_score (activated before deflation)
            (daa_score - self.deflationary_phase_daa_score) / self.bps_history.after()
        } else if daa_score < self.crescendo_activation_daa_score {
            // deflationary_phase <= daa_score < crescendo_activation (pre activation)
            (daa_score - self.deflationary_phase_daa_score) / self.bps_history.before()
        } else {
            // Else - deflationary_phase <= crescendo_activation <= daa_score.
            // Count seconds differently before and after Crescendo activation
            (self.crescendo_activation_daa_score - self.deflationary_phase_daa_score) / self.bps_history.before()
                + (daa_score - self.crescendo_activation_daa_score) / self.bps_history.after()
        };

        seconds_since_deflationary_phase_started / SECONDS_PER_MONTH
    }

    #[cfg(test)]
    pub fn legacy_calc_block_subsidy(&self, daa_score: u64) -> u64 {
        if daa_score < self.deflationary_phase_daa_score {
            return self.pre_deflationary_phase_base_subsidy;
        }

        // Note that this calculation implicitly assumes that block per second = 1 (by assuming daa score diff is in second units).
        let months_since_deflationary_phase_started = (daa_score - self.deflationary_phase_daa_score) / SECONDS_PER_MONTH;
        assert!(months_since_deflationary_phase_started <= usize::MAX as u64);
        let months_since_deflationary_phase_started: usize = months_since_deflationary_phase_started as usize;
        if months_since_deflationary_phase_started >= SUBSIDY_BY_MONTH_TABLE.len() {
            *SUBSIDY_BY_MONTH_TABLE.last().unwrap()
        } else {
            SUBSIDY_BY_MONTH_TABLE[months_since_deflationary_phase_started]
        }
    }
}

/*
    Marigold's own subsidy schedule (P1.4/P3.2 — see docs/marigold/DECISIONS.md and NOTES.md):
    210,000,000 MAGLD hard cap, no pre-deflationary phase (decay starts at block 0), smooth
    continuous geometric decay halving every 3 years (36 months), no tail — the table tapers
    to an exact 0 and stays there.

    Each entry is subsidy(month) = round(BASE * 0.5^(month / 36)), where BASE (petals/second,
    i.e. reward per block at a reference rate of 1 BPS) is the largest value for which
    Σ table[i] * SECONDS_PER_MONTH does not exceed the cap (210,000,000 * 10^8 petals),
    found by bisection. BASE ≈ 152,280,842.63 petals/sec ≈ 1.5228 MAGLD/sec.

    To regenerate this table, run:
    `cargo test --release --package kaspa-consensus --lib -- processes::coinbase::tests::generate_subsidy_table --exact --nocapture --ignored`
    (mirrors Kaspa's original convention of a `#[ignore]`d, manually-run table generator —
    see `total_emission_stays_under_cap` below for the permanent enforcement of the cap.)
*/
#[rustfmt::skip]
const SUBSIDY_BY_MONTH_TABLE: [u64; 1016] = [
	152280843, 149376860, 146528257, 143733976, 140992981, 138304258, 135666807, 133079653, 130541836, 128052415, 125610466, 123215086, 120865385, 118560493, 116299554,
	114081732, 111906203, 109772162, 107678816, 105625391, 103611124, 101635269, 99697093, 97795878, 95930920, 94101525, 92307017, 90546731, 88820013, 87126223,
	85464733, 83834928, 82236204, 80667966, 79129635, 77620640, 76140421, 74688430, 73264128, 71866988, 70496491, 69152129, 67833404, 66539827, 65270918,
	64026207, 62805233, 61607543, 60432692, 59280246, 58149777, 57040866, 55953102, 54886081, 53839408, 52812695, 51805562, 50817634, 49848547, 48897939,
	47965460, 47050763, 46153509, 45273365, 44410006, 43563111, 42732367, 41917464, 41118102, 40333983, 39564818, 38810320, 38070211, 37344215, 36632064,
	35933494, 35248245, 34576064, 33916702, 33269913, 32635459, 32013104, 31402617, 30803771, 30216346, 29640123, 29074889, 28520433, 27976551, 27443040,
	26919704, 26406348, 25902781, 25408817, 24924273, 24448970, 23982730, 23525381, 23076754, 22636683, 22205003, 21781556, 21366183, 20958732, 20559051,
	20166992, 19782409, 19405160, 19035105, 18672108, 18316032, 17966747, 17624123, 17288032, 16958351, 16634957, 16317730, 16006552, 15701308, 15401886,
	15108173, 14820062, 14537444, 14260217, 13988275, 13721520, 13459852, 13203174, 12951390, 12704409, 12462137, 12224485, 11991365, 11762691, 11538377,
	11318341, 11102502, 10890778, 10683092, 10479366, 10279525, 10083496, 9891204, 9702580, 9517553, 9336054, 9158016, 8983373, 8812061, 8644016,
	8479175, 8317478, 8158865, 8003276, 7850654, 7700943, 7554087, 7410031, 7268722, 7130108, 6994138, 6860760, 6729926, 6601587, 6475695,
	6352204, 6231068, 6112242, 5995682, 5881345, 5769189, 5659171, 5551251, 5445389, 5341546, 5239683, 5139763, 5041748, 4945602, 4851290,
	4758776, 4668027, 4579008, 4491687, 4406031, 4322008, 4239588, 4158739, 4079432, 4001638, 3925327, 3850471, 3777043, 3705015, 3634361,
	3565054, 3497069, 3430380, 3364963, 3300793, 3237848, 3176102, 3115534, 3056121, 2997841, 2940673, 2884594, 2829585, 2775625, 2722694,
	2670773, 2619842, 2569881, 2520874, 2472801, 2425645, 2379388, 2334013, 2289504, 2245843, 2203015, 2161004, 2119794, 2079370, 2039716,
	2000819, 1962664, 1925236, 1888522, 1852508, 1817181, 1782527, 1748534, 1715190, 1682482, 1650397, 1618924, 1588051, 1557767, 1528061,
	1498921, 1470336, 1442297, 1414793, 1387813, 1361347, 1335386, 1309921, 1284941, 1260437, 1236401, 1212823, 1189694, 1167007, 1144752,
	1122922, 1101508, 1080502, 1059897, 1039685, 1019858, 1000409, 981332, 962618, 944261, 926254, 908590, 891264, 874267, 857595,
	841241, 825198, 809462, 794026, 778884, 764030, 749460, 735168, 721149, 707396, 693906, 680674, 667693, 654960, 642470,
	630218, 618200, 606411, 594847, 583503, 572376, 561461, 550754, 540251, 529948, 519842, 509929, 500205, 490666, 481309,
	472130, 463127, 454295, 445632, 437134, 428798, 420620, 412599, 404731, 397013, 389442, 382015, 374730, 367584, 360574,
	353698, 346953, 340337, 333847, 327480, 321235, 315109, 309100, 303206, 297424, 291752, 286188, 280730, 275377, 270126,
	264974, 259921, 254965, 250102, 245333, 240654, 236065, 231563, 227148, 222816, 218567, 214399, 210310, 206300, 202365,
	198506, 194721, 191008, 187365, 183792, 180287, 176849, 173477, 170168, 166923, 163740, 160618, 157555, 154550, 151603,
	148712, 145876, 143094, 140365, 137688, 135063, 132487, 129961, 127482, 125051, 122666, 120327, 118033, 115782, 113574,
	111408, 109283, 107199, 105155, 103150, 101183, 99253, 97360, 95504, 93683, 91896, 90144, 88425, 86738, 85084,
	83462, 81870, 80309, 78777, 77275, 75801, 74356, 72938, 71547, 70183, 68844, 67531, 66244, 64980, 63741,
	62526, 61333, 60164, 59016, 57891, 56787, 55704, 54642, 53600, 52578, 51575, 50591, 49627, 48680, 47752,
	46841, 45948, 45072, 44212, 43369, 42542, 41731, 40935, 40154, 39389, 38638, 37901, 37178, 36469, 35774,
	35091, 34422, 33766, 33122, 32490, 31871, 31263, 30667, 30082, 29508, 28945, 28393, 27852, 27321, 26800,
	26289, 25787, 25296, 24813, 24340, 23876, 23421, 22974, 22536, 22106, 21685, 21271, 20865, 20468, 20077,
	19694, 19319, 18950, 18589, 18234, 17887, 17546, 17211, 16883, 16561, 16245, 15935, 15631, 15333, 15041,
	14754, 14473, 14197, 13926, 13660, 13400, 13144, 12894, 12648, 12407, 12170, 11938, 11710, 11487, 11268,
	11053, 10842, 10636, 10433, 10234, 10039, 9847, 9659, 9475, 9294, 9117, 8943, 8773, 8606, 8441,
	8280, 8123, 7968, 7816, 7667, 7520, 7377, 7236, 7098, 6963, 6830, 6700, 6572, 6447, 6324,
	6203, 6085, 5969, 5855, 5744, 5634, 5527, 5421, 5318, 5216, 5117, 5019, 4924, 4830, 4738,
	4647, 4559, 4472, 4386, 4303, 4221, 4140, 4061, 3984, 3908, 3833, 3760, 3689, 3618, 3549,
	3481, 3415, 3350, 3286, 3223, 3162, 3102, 3043, 2984, 2928, 2872, 2817, 2763, 2711, 2659,
	2608, 2558, 2510, 2462, 2415, 2369, 2324, 2279, 2236, 2193, 2151, 2110, 2070, 2031, 1992,
	1954, 1917, 1880, 1844, 1809, 1775, 1741, 1708, 1675, 1643, 1612, 1581, 1551, 1521, 1492,
	1464, 1436, 1408, 1382, 1355, 1329, 1304, 1279, 1255, 1231, 1207, 1184, 1162, 1140, 1118,
	1097, 1076, 1055, 1035, 1015, 996, 977, 958, 940, 922, 905, 887, 870, 854, 837,
	822, 806, 790, 775, 761, 746, 732, 718, 704, 691, 678, 665, 652, 640, 627,
	615, 604, 592, 581, 570, 559, 548, 538, 528, 518, 508, 498, 488, 479, 470,
	461, 452, 444, 435, 427, 419, 411, 403, 395, 388, 380, 373, 366, 359, 352,
	345, 339, 332, 326, 320, 314, 308, 302, 296, 290, 285, 279, 274, 269, 264,
	259, 254, 249, 244, 240, 235, 231, 226, 222, 218, 213, 209, 205, 201, 198,
	194, 190, 187, 183, 179, 176, 173, 169, 166, 163, 160, 157, 154, 151, 148,
	145, 142, 140, 137, 134, 132, 129, 127, 124, 122, 120, 118, 115, 113, 111,
	109, 107, 105, 103, 101, 99, 97, 95, 93, 91, 90, 88, 86, 85, 83,
	82, 80, 78, 77, 75, 74, 73, 71, 70, 69, 67, 66, 65, 63, 62,
	61, 60, 59, 58, 57, 55, 54, 53, 52, 51, 50, 49, 48, 48, 47,
	46, 45, 44, 43, 42, 42, 41, 40, 39, 38, 38, 37, 36, 36, 35,
	34, 34, 33, 32, 32, 31, 31, 30, 29, 29, 28, 28, 27, 27, 26,
	26, 25, 25, 24, 24, 23, 23, 22, 22, 22, 21, 21, 20, 20, 20,
	19, 19, 19, 18, 18, 17, 17, 17, 16, 16, 16, 16, 15, 15, 15,
	14, 14, 14, 14, 13, 13, 13, 13, 12, 12, 12, 12, 11, 11, 11,
	11, 11, 10, 10, 10, 10, 10, 9, 9, 9, 9, 9, 9, 8, 8,
	8, 8, 8, 8, 7, 7, 7, 7, 7, 7, 7, 7, 6, 6, 6,
	6, 6, 6, 6, 6, 6, 5, 5, 5, 5, 5, 5, 5, 5, 5,
	5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3,
	3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
	3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
	2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1,
	1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
	1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
	1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
	1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::MAINNET_PARAMS;
    use kaspa_consensus_core::{
        config::params::{ForkActivation, Params, SIMNET_PARAMS},
        constants::SOMPI_PER_KASPA,
        network::{NetworkId, NetworkType},
        tx::scriptvec,
    };

    /// P3.2: regenerates [`SUBSIDY_BY_MONTH_TABLE`] from the P1.4 economics decision (210,000,000
    /// MAGLD cap, 3-year/36-month halving, smooth continuous decay, no pre-deflationary phase).
    /// Not run by default — this documents/reproduces how the table was derived; the table
    /// itself is a frozen, pasted-in const (same convention Kaspa's own generator used). Run
    /// manually with:
    /// `cargo test --release --package kaspa-consensus --lib -- processes::coinbase::tests::generate_subsidy_table --exact --nocapture --ignored`
    #[test]
    #[ignore = "run manually to (re)generate SUBSIDY_BY_MONTH_TABLE"]
    fn generate_subsidy_table() {
        const CAP_PETALS: u128 = 210_000_000 * SOMPI_PER_KASPA as u128;
        const HALVING_MONTHS: f64 = 36.0;

        let build_table = |base: f64| -> Vec<u64> {
            let mut table = Vec::new();
            let mut month = 0u64;
            loop {
                let val = (base * 0.5f64.powf(month as f64 / HALVING_MONTHS)).round();
                if val <= 0.0 {
                    table.push(0);
                    break;
                }
                table.push(val as u64);
                month += 1;
            }
            table
        };

        let total_emission = |table: &[u64]| -> u128 { table.iter().map(|&v| v as u128 * SECONDS_PER_MONTH as u128).sum() };

        let halving_seconds = HALVING_MONTHS * SECONDS_PER_MONTH as f64;
        let base_estimate = CAP_PETALS as f64 * std::f64::consts::LN_2 / halving_seconds;

        // Bisect for the largest BASE whose discrete, rounded table still sums to <= cap.
        let (mut lo, mut hi) = (0.0f64, base_estimate * 1.05);
        for _ in 0..200 {
            let mid = (lo + hi) / 2.0;
            if total_emission(&build_table(mid)) <= CAP_PETALS {
                lo = mid;
            } else {
                hi = mid;
            }
        }

        let table = build_table(lo);
        let total = total_emission(&table);
        assert!(total <= CAP_PETALS);
        assert_eq!(*table.last().unwrap(), 0, "table must taper to an exact 0 (no tail)");

        println!(
            "base = {} petals/sec ({} MAGLD/sec), len = {}, total = {}, cap = {}, slack = {}",
            lo,
            lo / SOMPI_PER_KASPA as f64,
            table.len(),
            total,
            CAP_PETALS,
            CAP_PETALS - total
        );
        for chunk in table.chunks(15) {
            let row: String = chunk.iter().map(|v| format!("{v}, ")).collect();
            println!("\t{}", row.trim_end());
        }
    }

    /// P3.2: the P1.4 economics decision (210,000,000 MAGLD cap, no tail emission) is enforced
    /// by construction — this must keep passing for every future edit to
    /// [`SUBSIDY_BY_MONTH_TABLE`]. This is the "provably sums below the cap" verification the
    /// Phase 3 goal in FORK-PLAN.md asks for.
    #[test]
    fn total_emission_stays_under_cap() {
        const CAP_PETALS: u128 = 210_000_000 * SOMPI_PER_KASPA as u128;
        let total: u128 = SUBSIDY_BY_MONTH_TABLE.iter().map(|&v| v as u128 * SECONDS_PER_MONTH as u128).sum();
        assert!(total <= CAP_PETALS, "total emission {total} petals exceeds the 210,000,000 MAGLD cap ({CAP_PETALS} petals)");
        assert_eq!(*SUBSIDY_BY_MONTH_TABLE.last().unwrap(), 0, "table must taper to an exact 0 (no tail emission)");
        // Sanity: no reasonable cap should be missed by more than a rounding hair (the actual
        // slack here is ~0.0036 MAGLD out of 210,000,000 — this bound is deliberately loose).
        assert!(CAP_PETALS - total < SOMPI_PER_KASPA as u128, "unexpectedly large slack between total emission and the cap");
    }

    #[test]
    fn calc_high_bps_total_rewards_delta() {
        let legacy_cbm = create_legacy_manager();
        let pre_deflationary_rewards = legacy_cbm.pre_deflationary_phase_base_subsidy * legacy_cbm.deflationary_phase_daa_score;
        let total_rewards: u64 = pre_deflationary_rewards + SUBSIDY_BY_MONTH_TABLE.iter().map(|x| x * SECONDS_PER_MONTH).sum::<u64>();
        let testnet_11_bps = SIMNET_PARAMS.bps();
        let total_high_bps_rewards_rounded_up: u64 = pre_deflationary_rewards
            + SUBSIDY_BY_MONTH_TABLE.iter().map(|x| (x.div_ceil(testnet_11_bps) * testnet_11_bps) * SECONDS_PER_MONTH).sum::<u64>();

        let cbm = create_manager(&SIMNET_PARAMS);
        let total_high_bps_rewards: u64 = pre_deflationary_rewards
            + cbm.subsidy_by_month_table_before.iter().map(|x| x * SECONDS_PER_MONTH * cbm.bps().before()).sum::<u64>();
        assert_eq!(total_high_bps_rewards_rounded_up, total_high_bps_rewards, "subsidy adjusted to bps must be rounded up");

        let delta = total_high_bps_rewards as i64 - total_rewards as i64;

        println!("Total rewards: {} sompi => {} KAS", total_rewards, total_rewards / SOMPI_PER_KASPA);
        println!("Total high bps rewards: {} sompi => {} KAS", total_high_bps_rewards, total_high_bps_rewards / SOMPI_PER_KASPA);
        println!("Delta: {} sompi => {} KAS", delta, delta / SOMPI_PER_KASPA as i64);
    }

    #[test]
    fn subsidy_by_month_table_test() {
        let cbm = create_legacy_manager();
        cbm.subsidy_by_month_table_before.iter().enumerate().for_each(|(i, x)| {
            assert_eq!(SUBSIDY_BY_MONTH_TABLE[i], *x, "for 1 BPS, const table and precomputed values must match");
        });

        for network_id in NetworkId::iter() {
            let cbm = create_manager(&network_id.into());
            cbm.subsidy_by_month_table_before.iter().enumerate().for_each(|(i, x)| {
                assert_eq!(
                    SUBSIDY_BY_MONTH_TABLE[i].div_ceil(cbm.bps().before()),
                    *x,
                    "{}: locally computed and precomputed values must match",
                    network_id
                );
            });
            cbm.subsidy_by_month_table_after.iter().enumerate().for_each(|(i, x)| {
                assert_eq!(
                    SUBSIDY_BY_MONTH_TABLE[i].div_ceil(cbm.bps().after()),
                    *x,
                    "{}: locally computed and precomputed values must match",
                    network_id
                );
            });
        }
    }

    /// Takes over 60 seconds, run with the following command line:
    /// `cargo test --release --package kaspa-consensus --lib -- processes::coinbase::tests::verify_crescendo_emission_schedule --exact --nocapture --ignored`
    #[test]
    #[ignore = "long"]
    fn verify_crescendo_emission_schedule() {
        // No need to loop over all nets since the relevant params are only
        // deflation and activation DAA scores (and the test is long anyway)
        for network_id in [NetworkId::new(NetworkType::Mainnet)] {
            let mut params: Params = network_id.into();
            params.crescendo_activation = ForkActivation::never();
            let cbm = create_manager(&params);
            let (baseline_epochs, baseline_total) = calculate_emission(cbm);

            let mut activations = vec![10000, 33444444, 120727479];
            for network_id in NetworkId::iter() {
                let activation = Params::from(network_id).crescendo_activation;
                if activation != ForkActivation::never() && activation != ForkActivation::always() {
                    activations.push(activation.daa_score());
                }
            }

            // Loop over a few random activation points + specified activation points for all nets
            for activation in activations {
                params.crescendo_activation = ForkActivation::new(activation);
                let cbm = create_manager(&params);
                let (new_epochs, new_total) = calculate_emission(cbm);

                // Epochs only represents the number of times the subsidy changed (lower after activation due to rounding)
                println!("BASELINE:\t{}\tepochs, total emission: {}", baseline_epochs, baseline_total);
                println!("CRESCENDO:\t{}\tepochs, total emission: {}, activation: {}", new_epochs, new_total, activation);

                let diff = (new_total as i64 - baseline_total as i64) / SOMPI_PER_KASPA as i64;
                assert!(diff.abs() <= 51, "activation: {}", activation);
                println!("DIFF (KAS): {}", diff);
            }
        }
    }

    fn calculate_emission(cbm: CoinbaseManager) -> (u64, u64) {
        let activation = cbm.bps().activation().daa_score();
        let mut current = 0;
        let mut total = 0;
        let mut epoch = 0u64;
        let mut prev = cbm.calc_block_subsidy(0);
        loop {
            let subsidy = cbm.calc_block_subsidy(current);
            // `legacy_calc_block_subsidy` treats its argument as literal elapsed seconds (1-BPS
            // reference), so `current` (real blocks) must first be converted to seconds via the
            // actual pre-crescendo BPS before calling it, and its raw (unscaled) table value then
            // divided by that same BPS to compare against calc_block_subsidy's real per-block
            // subsidy. Real Kaspa's original version of this assertion skipped both conversions,
            // since their pre_crescendo_target_time_per_block really was 1 BPS (both are no-ops
            // at bps=1); Marigold's is deliberately equal to the post-crescendo rate (P2.2 — no
            // real pre-crescendo history to protect on a from-scratch chain), so both matter here.
            if current < activation {
                let bps_before = cbm.bps().before();
                assert_eq!(cbm.legacy_calc_block_subsidy(current / bps_before).div_ceil(bps_before), subsidy);
            }
            if subsidy == 0 {
                break;
            }
            total += subsidy;
            if subsidy != prev {
                println!("epoch: {}, subsidy: {}", epoch, subsidy);
                prev = subsidy;
                epoch += 1;
            }
            current += 1;
        }

        (epoch, total)
    }

    /// Unlike upstream Kaspa's version of this test (which cross-checked hardcoded fractions of
    /// the initial subsidy against specific halving counts tuned to Kaspa's own 426-entry/
    /// 12-month-halving table), this spot-checks `calc_block_subsidy`'s DAA-score -> month ->
    /// table-lookup -> BPS-scaling wiring directly against real [`SUBSIDY_BY_MONTH_TABLE`]
    /// entries. That's a deliberate choice: `table[i] / 2^n` is not exactly equal to
    /// `round(BASE * 0.5^((i + n*36)/36))` in general (confirmed empirically while building the
    /// table — they agree at n=1 and n=5 but drift by 1 unit at n=2), so re-deriving expectations
    /// via a second formula would be fragile. The table's own correctness (shape, exact-zero
    /// tail, cap) is separately covered by `total_emission_stays_under_cap` and
    /// `generate_subsidy_table`.
    #[test]
    fn subsidy_test() {
        // Mainnet/testnet/devnet have no pre-deflationary phase (P2.6/P3.2 — deflationary_phase_daa_score
        // == 0, decay starts at block 0). Simnet is the one exception, deliberately (see
        // SIMNET_PARAMS' comment in params.rs): it's a PoW-skipped internal benchmark/test
        // harness, not a real network, and daemon_integration_tests.rs relies on it keeping a
        // real flat pre-deflationary subsidy for `coinbase_maturity` blocks. Table-driven month
        // checks below are offset by `params.deflationary_phase_daa_score`, which correctly
        // generalizes across both cases. Real Kaspa's 1-BPS-reference/BPS-scaling math itself is
        // cross-checked separately, via `create_legacy_manager()`, in
        // `calc_high_bps_total_rewards_delta` / `subsidy_by_month_table_test`.
        const HALVING_PERIOD_MONTHS: u64 = 36;
        let last_nonzero_month = (SUBSIDY_BY_MONTH_TABLE.len() - 2) as u64;
        let last_zero_month = (SUBSIDY_BY_MONTH_TABLE.len() - 1) as u64;

        for network_id in NetworkId::iter() {
            let mut params: Params = network_id.into();
            if params.crescendo_activation != ForkActivation::always() {
                // We test activation scenarios in verify_crescendo_emission_schedule
                params.crescendo_activation = ForkActivation::never();
            }

            let cbm = create_manager(&params);
            let bps = params.bps_history().after();
            let blocks_per_month = SECONDS_PER_MONTH * bps;

            if params.deflationary_phase_daa_score > 0 {
                assert_eq!(
                    cbm.calc_block_subsidy(1),
                    params.pre_deflationary_phase_base_subsidy,
                    "{}: first mined block should be pre-deflationary",
                    network_id
                );
                assert_eq!(
                    cbm.calc_block_subsidy(params.deflationary_phase_daa_score - 1),
                    params.pre_deflationary_phase_base_subsidy,
                    "{}: last block before the deflationary phase",
                    network_id
                );
            }

            struct Test {
                name: &'static str,
                month: u64,
            }

            let tests = [
                Test { name: "start of deflationary phase", month: 0 },
                Test { name: "one month in", month: 1 },
                Test { name: "two months in", month: 2 },
                Test { name: "five months in", month: 5 },
                Test { name: "after one halving", month: HALVING_PERIOD_MONTHS },
                Test { name: "after two halvings", month: 2 * HALVING_PERIOD_MONTHS },
                Test { name: "last nonzero month", month: last_nonzero_month },
                Test { name: "subsidy depleted", month: last_zero_month },
                Test { name: "long past depletion", month: last_zero_month + 10_000 },
            ];

            for t in tests {
                let table_index = (t.month as usize).min(SUBSIDY_BY_MONTH_TABLE.len() - 1);
                let expected = SUBSIDY_BY_MONTH_TABLE[table_index].div_ceil(bps);
                let daa_score = params.deflationary_phase_daa_score + t.month * blocks_per_month;
                assert_eq!(cbm.calc_block_subsidy(daa_score), expected, "{} test '{}' failed", network_id, t.name);
                if bps == 1 {
                    assert_eq!(cbm.legacy_calc_block_subsidy(daa_score), expected, "{} test '{}' failed", network_id, t.name);
                }
            }
        }
    }

    #[test]
    fn payload_serialization_test() {
        let cbm = create_manager(&MAINNET_PARAMS);

        let script_data = [33u8, 255];
        let extra_data = [2u8, 3];
        let data = CoinbaseData {
            blue_score: 56,
            subsidy: 44000000000,
            miner_data: MinerData {
                script_public_key: ScriptPublicKey::new(0, ScriptVec::from_slice(&script_data)),
                extra_data: &extra_data as &[u8],
            },
        };

        let payload = cbm.serialize_coinbase_payload(&data).unwrap();
        let deserialized_data = cbm.deserialize_coinbase_payload(&payload).unwrap();

        assert_eq!(data, deserialized_data);

        // Test an actual mainnet payload
        let payload_hex =
            "b612c90100000000041a763e07000000000022202b32443ff740012157716d81216d09aebc39e5493c93a7181d92cb756c02c560ac302e31322e382f";
        let mut payload = vec![0u8; payload_hex.len() / 2];
        faster_hex::hex_decode(payload_hex.as_bytes(), &mut payload).unwrap();
        let deserialized_data = cbm.deserialize_coinbase_payload(&payload).unwrap();

        let expected_data = CoinbaseData {
            blue_score: 29954742,
            subsidy: 31112698372,
            miner_data: MinerData {
                script_public_key: ScriptPublicKey::new(
                    0,
                    scriptvec![
                        32, 43, 50, 68, 63, 247, 64, 1, 33, 87, 113, 109, 129, 33, 109, 9, 174, 188, 57, 229, 73, 60, 147, 167, 24,
                        29, 146, 203, 117, 108, 2, 197, 96, 172,
                    ],
                ),
                extra_data: &[48u8, 46, 49, 50, 46, 56, 47] as &[u8],
            },
        };
        assert_eq!(expected_data, deserialized_data);
    }

    #[test]
    fn modify_payload_test() {
        let cbm = create_manager(&MAINNET_PARAMS);

        let script_data = [33u8, 255];
        let extra_data = [2u8, 3, 23, 98];
        let data = CoinbaseData {
            blue_score: 56345,
            subsidy: 44000000000,
            miner_data: MinerData {
                script_public_key: ScriptPublicKey::new(0, ScriptVec::from_slice(&script_data)),
                extra_data: &extra_data,
            },
        };

        let data2 = CoinbaseData {
            blue_score: data.blue_score,
            subsidy: data.subsidy,
            miner_data: MinerData {
                // Modify only miner data
                script_public_key: ScriptPublicKey::new(0, ScriptVec::from_slice(&[33u8, 255, 33])),
                extra_data: &[2u8, 3, 23, 98, 34, 34] as &[u8],
            },
        };

        let mut payload = cbm.serialize_coinbase_payload(&data).unwrap();
        payload = cbm.modify_coinbase_payload(payload, &data2.miner_data).unwrap(); // Update the payload with the modified miner data
        let deserialized_data = cbm.deserialize_coinbase_payload(&payload).unwrap();

        assert_eq!(data2, deserialized_data);
    }

    #[test]
    fn expected_coinbase_transaction_selects_version_by_toccata_activation() {
        let mut params = MAINNET_PARAMS.clone();
        params.toccata_activation = ForkActivation::new(100);
        let cbm = create_manager(&params);
        let miner_data = MinerData::new(ScriptPublicKey::new(0, scriptvec![1, 2, 3]), vec![4, 5, 6]);
        let ghostdag_data = GhostdagData::default();
        let mergeset_rewards = Default::default();
        let mergeset_non_daa = Default::default();

        let pre_activation =
            cbm.expected_coinbase_transaction(99, miner_data.clone(), &ghostdag_data, &mergeset_rewards, &mergeset_non_daa).unwrap();
        let post_activation =
            cbm.expected_coinbase_transaction(100, miner_data, &ghostdag_data, &mergeset_rewards, &mergeset_non_daa).unwrap();

        assert_eq!(pre_activation.tx.version, constants::TX_VERSION);
        assert_eq!(post_activation.tx.version, constants::TX_VERSION_TOCCATA);
    }

    fn create_manager(params: &Params) -> CoinbaseManager {
        CoinbaseManager::new(
            params.coinbase_payload_script_public_key_max_len,
            params.max_coinbase_payload_len,
            params.deflationary_phase_daa_score,
            params.pre_deflationary_phase_base_subsidy,
            params.bps_history(),
            params.toccata_activation,
        )
    }

    /// Return a CoinbaseManager with legacy golang 1 BPS properties
    fn create_legacy_manager() -> CoinbaseManager {
        CoinbaseManager::new(150, 204, 15778800 - 259200, 50000000000, ForkedParam::new_const(1), ForkActivation::never())
    }
}

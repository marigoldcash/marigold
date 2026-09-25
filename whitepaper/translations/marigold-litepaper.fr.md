# Marigold — Livre blanc abrégé

## Le problème : la monnaie numérique a oublié ce que l'argent liquide faisait bien

Sortez un billet de votre portefeuille. Remarquez trois choses :

1. **Celui qui le détient le possède.** Pas de compte, pas d'identifiant, pas d'intermédiaire. La possession vaut propriété.
2. **Il n'a aucune mémoire.** Le billet ne se souvient pas de qui l'a dépensé en dernier. Aucune trace ne vous relie au café que vous avez acheté ce matin.
3. **N'importe qui peut vérifier qu'il est authentique.** On le regarde à la lumière, on tâte le papier. Aucune expertise particulière.

Aucune monnaie numérique grand public ne réunit ces trois propriétés à la fois.

Bitcoin vous offre la première et la troisième : détenir vos clés, c'est posséder vos pièces, et n'importe qui peut vérifier que le système est honnête. Mais il échoue lourdement sur la deuxième. Chaque bitcoin jamais miné traîne son historique complet, en permanence et en public. Chaque adresse depuis laquelle vous avez envoyé ou reçu se retrouve cousue à toutes les autres dans un graphe que n'importe qui peut analyser. Vos pièces se souviennent de tout ce que vous avez fait avec elles.

Les cryptomonnaies anonymes comme Monero et Zcash tentent d'y remédier en chiffrant l'historique. Elles réussissent à cacher — mais la cryptographie même qui permet de cacher crée deux problèmes nouveaux. D'abord, presque personne n'est en mesure de vérifier par soi-même que le système est honnête. Vous faites confiance à des experts que vous n'avez jamais rencontrés pour vous assurer que les mathématiques ne contiennent ni faille ni porte dérobée. Ensuite, les régulateurs jettent un coup d'œil à cette cryptographie complexe et classent la monnaie en conséquence : retraits de cotation, restrictions et traitement hostile découlent non pas de ce que fait la monnaie, mais de la façon dont elle le fait.

Résultat : un paysage où il faut choisir entre la transparence sans vie privée et la vie privée sans confiance. L'argent liquide offrait les deux, sans effort. La monnaie numérique a fini par oublier comment.

---

## L'idée : ne pas cacher la trace — ne pas en laisser

Imaginez-vous devant un étal de marché. Vous tendez un billet de cinq dollars au marchand. Le marchand le met dans sa caisse. Plus tard dans la journée, il dépense ce même billet à la boulangerie. La boulangerie le rend en monnaie au client suivant.

Posez-vous la question : qui peut remonter de ce billet jusqu'à vous ? Personne. Non pas parce que le trajet du billet était chiffré ou dissimulé — le billet est resté visible de tous à chaque étape — mais parce que le billet lui-même n'a jamais enregistré qui le remettait à qui. Le lien entre vous et le marchand n'a jamais été écrit nulle part. Il a existé le temps de la remise, puis il a disparu.

Voilà toute la différence entre cacher une information et ne jamais la recueillir. Les cryptomonnaies anonymes cachent. Marigold ne recueille rien.

---

## Comment ça marche : des billets, pas des comptes

La plupart des cryptomonnaies fonctionnent comme des comptes bancaires. Vous avez une adresse (l'équivalent d'un numéro de compte) et les transactions déplacent de la valeur d'une adresse à l'autre. Les adresses sont publiques et chaque mouvement entre elles est enregistré pour toujours. Un rêve de comptable, un cauchemar pour la vie privée.

Marigold fonctionne comme l'argent liquide. Il n'y a pas de comptes. Le système tient à la place un ensemble de **billets** — voyez-les comme des billets de banque numériques.

Chaque billet est un enregistrement public tout simple :

- Un **numéro de série** (comme celui imprimé sur un billet de banque)
- Une **valeur** (0,01, 0,1, 1, 10, 100, 1 000, 10 000 ou 100 000 MAGLD)
- Une **serrure** (une clé publique — qui détient la clé correspondante peut le dépenser)

C'est tout. Pas de nom de propriétaire, pas d'adresse, pas d'identité. Le billet ne sait pas qui le détient. Il sait seulement qu'une clé existe et que celui qui parvient à l'ouvrir peut le dépenser.

### Payer quelqu'un

Payer quelqu'un, c'est comme tendre un billet, avec une étape en plus :

1. **Vous donnez au destinataire la clé** du billet (par QR code, par SMS, ou même à l'oral — comme vous partageriez une photo).
2. **Le destinataire change aussitôt la serrure** — il inscrit une transaction qui remplace votre clé par une clé toute neuve que lui seul connaît. C'est confirmé en moins d'une seconde.

Une fois ce changement confirmé, le billet lui appartient irrévocablement. Vous ne détenez plus de clé qui fonctionne. Le règlement est terminé.

Autre possibilité : si le destinataire peut vous transmettre sa nouvelle clé à l'avance (par une demande de paiement, par exemple), vous changez la serrure directement pour la sienne. Votre clé ne circule nulle part.

La troisième manière sert à payer quelqu'un qui n'est pas là. Vous changez la serrure pour une clé que lui seul détient, et vous y mettez une échéance. Jusqu'à l'échéance, lui seul peut prendre le billet ; à partir de l'échéance, vous seul pouvez le reprendre. Il l'encaisse quand il veut ; s'il ne le fait jamais, l'argent redevient le vôtre de lui-même. L'échéance est imposée par le réseau, pas par le portefeuille de qui que ce soit.

Les trois manières finissent de la même façon : le billet est sous une clé que seul le destinataire connaît.

### Diviser et fusionner

Les billets existent en valeurs fixes, exactement comme les espèces. Si vous devez payer 30 MAGLD et que vous détenez un billet de 100 MAGLD, vous le **divisez** en dix billets de 10 MAGLD. Vous en tendez trois. Vous en gardez sept. Faire la monnaie — comme avec l'argent liquide.

Dans l'autre sens, dix billets de 10 MAGLD peuvent être **fusionnés** en un seul billet de 100 MAGLD. Les valeurs vont de dix en dix, si bien que n'importe quel montant se paie avec une petite poignée de billets.

### Ce que le système enregistre

Toute opération — créer un billet, changer une serrure, diviser, fusionner, encaisser — est publique et visible de tous. Ce que personne ne peut voir, c'est **qui** l'a faite, parce que le système n'a aucune notion de « qui ». Pas de champ expéditeur. Pas de champ destinataire. Pas de carnet d'adresses. Pas de compte.

Le système enregistre qu'un billet a changé de mains. Il n'enregistre pas de quelles mains. Un billet sous échéance montre aussi ses conditions — l'échéance, et la clé vers laquelle il revient — tant que l'échéance court.

---

## Marigold dans la vie de tous les jours

Parce qu'un billet Marigold n'est rien d'autre qu'une clé, s'en servir dans la vie réelle est aussi simple que de se servir d'argent liquide. Voici comment :

### Imprimez-le, dépensez-le

Chaque billet de votre application de portefeuille peut s'afficher sous forme de **QR code**. Vous pouvez imprimer ce QR code sur une feuille de papier, la plier et la glisser dans votre portefeuille en cuir — juste à côté de vos cartes bancaires et de votre permis de conduire. Ce bout de papier *est* l'argent. Montrez-le à quelqu'un : il le scanne, change la serrure, et le billet est à lui. Vous ne détenez plus de clé qui fonctionne. Le papier que vous avez en poche ne vaut plus rien — la valeur est passée entre ses mains.

Ce n'est pas une métaphore. Le QR code imprimé contient la clé elle-même. Le perdre, c'est comme perdre un billet de cent dollars : celui qui le trouve peut le dépenser. Le garder en sécurité est exactement la même responsabilité que garder de l'argent liquide en sécurité. Il n'y a pas de numéro d'assistance, pas de bouton « signaler un vol », pas de banque pour annuler la transaction. C'est cela, être au porteur, et c'est tout l'intérêt.

### Offrez-le

Une carte d'anniversaire avec un QR code imprimé à l'intérieur ne se distingue en rien d'une carte d'anniversaire contenant un billet de cinquante dollars. Le destinataire le scanne, change aussitôt la serrure, et le billet est irrévocablement à lui. Aucun compte à ouvrir, aucun délai d'attente. Il ouvre l'enveloppe, il a l'argent en main.

### Laissez un héritage

Une enveloppe scellée dans un coffre, contenant les QR codes imprimés de plusieurs billets, fonctionne exactement comme une enveloppe de billets de banque. Qui l'ouvre détient les clés. Qui détient les clés détient les billets. Aucun exécuteur testamentaire, aucun tribunal des successions, aucune autorisation d'un tiers n'est nécessaire pour que la valeur change de main — même si, comme avec l'argent liquide, les dispositions *juridiques* de la succession sont une tout autre affaire, que le système ne règle pas et ne peut pas régler. Le système garantit une seule chose : détenir la clé, c'est détenir le billet.

### Payez sur un marché

Vous achetez des tomates. Le marchand affiche un QR code : sa demande de paiement. Vous le scannez avec votre application de portefeuille, vous choisissez le billet avec lequel vous voulez payer, et votre téléphone change la serrure directement pour la clé du marchand. Moins d'une seconde plus tard, le billet est à lui. Vous empochez vos tomates. Le marchand n'a jamais vu votre nom, votre adresse ni votre compte. Vous n'avez jamais vu les siens. La transaction est réglée, définitive et oubliable — comme avec de l'argent liquide.

### Être payé en votre absence

Publiez une seule clé — sur votre carte de visite, dans votre vitrine, sur votre profil — et n'importe qui peut vous payer à toute heure. Chaque paiement arrive sous une clé neuve que seul votre portefeuille sait déduire de celle que vous avez publiée ; la chaîne ne montre donc jamais deux paiements arrivant au même endroit. Un paiement vers cette clé peut porter une échéance, de sorte que l'expéditeur sait que l'argent lui revient si vous ne l'encaissez jamais. Votre portefeuille encaisse ce qui l'attend la prochaine fois qu'il tourne.

### Rangez-le hors ligne

Les pirates vous inquiètent ? Imprimez vos billets sous forme de QR codes, rangez-les dans une boîte ignifugée et supprimez l'application. Les billets existent sur la blockchain. Les clés existent sur papier. Aucun appareil connecté à Internet ne les détient. Le jour où vous voulez dépenser, rescannez le QR code dans une application de portefeuille, changez aussitôt la serrure (au cas où quelqu'un aurait copié le papier pendant qu'il dormait là) et payez normalement.

### Changez de portefeuille

Comme les billets sont des clés indépendantes, qui ne dépendent d'aucune phrase de récupération ni d'aucun compte, vous pouvez déplacer un billet d'une application de portefeuille à une autre quand bon vous semble. Exportez la clé d'une application, importez-la dans une autre. Aucune transaction sur la chaîne, aucun frais, aucune interaction avec le réseau. Votre billet fonctionne de la même façon dans tous les portefeuilles compatibles avec Marigold — choisissez l'application qui vous plaît, changez quand vous voulez, votre argent vous suit.

Le fil conducteur : un billet Marigold est une clé, et une clé peut s'imprimer, s'envoyer par SMS, se plier dans une enveloppe, s'aimanter sur un frigo ou s'apprendre par cœur. Le système se moque de la façon dont la clé passe d'une personne à l'autre, parce qu'il ignore que les personnes existent. Il ne connaît que des clés et des billets — et c'est pourquoi chaque manière de faire circuler un billet de banque a ici un équivalent numérique direct.

Et rien de tout cela n'exige de connexion. Le bitcoin peut être conservé hors ligne sous forme de code QR, mais pas dépensé ainsi : payer avec, c'est diffuser une transaction, donc quelqu'un doit être en ligne, et un portefeuille qui ne l'est jamais ne peut pas payer. Un billet Marigold est la clé elle-même : il peut passer de main en main hors ligne, exactement comme un billet de banque ; le réseau n'est touché que lorsque le nouveau détenteur décide de changer la serrure. Pas besoin d'être connecté pour payer, ni d'un portefeuille en ligne en permanence.

---

## Votre téléphone est une télécommande, pas un portefeuille

Tout ce que vous faites à votre clavier, vous pouvez le faire depuis votre téléphone, dans une conversation Telegram : consulter votre solde, payer quelqu'un, recevoir un billet, émettre une demande de paiement, relire votre historique, vérifier que votre mineur tourne. Ce qui change, ce n'est pas ce que vous pouvez faire. C'est l'endroit où se trouve l'argent — et l'argent n'est pas sur le téléphone.

Votre portefeuille tourne chez vous, sur ce qui reste allumé : un ordinateur portable au fond d'un tiroir, une petite machine posée à côté du routeur. Il se tient à jour avec la chaîne, il garde vos billets et il répond à vos messages. Le téléphone dans votre poche ne contient aucune clé et ne stocke aucun billet. Il ne parle à rien d'autre qu'à Telegram. Perdez-le : vous avez perdu une télécommande.

Le bot est le vôtre, pas le nôtre. Vous le créez dans Telegram en deux minutes environ, vous confiez le jeton à votre portefeuille, et vous appariez les deux avec un code. À partir de là, votre portefeuille répond à ce seul compte Telegram et ignore tous les autres. Aucun serveur Marigold au milieu, aucun compte chez nous, aucun parc de machines que nous ferions tourner et dont votre téléphone dépendrait. Votre portefeuille appelle Telegram ; rien ne l'appelle, et il n'y a aucun port à ouvrir sur votre routeur.

Les dépenses sont protégées comme celles d'une carte bancaire : un code avant toute opération qui déplace de l'argent, un blocage après trois erreurs que seule la machine restée chez vous peut lever, et un plafond quotidien que vous fixez vous-même. Ce compte Telegram peut désormais déplacer de l'argent : il lui faut donc une authentification à deux facteurs — le bot vous le dit dès votre premier échange avec lui.

Le même fil de discussion garde votre sauvegarde. Dès que vous le lui demandez une fois, le portefeuille y dépose une copie chiffrée de lui-même et la tient à jour tout seul — une copie complète chaque semaine, les changements quelques minutes après un paiement — en silence, sans notification. Si la machine à la maison meurt, vous transférez ces messages à un portefeuille neuf et tout est là. La copie est verrouillée par vos 24 mots et par rien d'autre : pas par le mot de passe du portefeuille, que vous tapez chaque jour et que vous avez choisi pour pouvoir le retenir, et que quiconque détient une copie du fichier pourrait essayer à loisir. Les 24 mots, eux, ne se devinent pas. Notez-les quand le portefeuille vous les montre — il vous en redemandera deux pour s'en assurer — et une sauvegarde sur le serveur de quelqu'un d'autre n'est que du bruit pour tout le monde sauf vous.

Les limites, sans détour. Quand la machine restée chez vous est éteinte, le téléphone ne peut plus rien faire : pas de solde, pas de paiement, pas de réception. Un code de paiement envoyé dans une conversation est une valeur au porteur tant qu'il circule, exactement comme les QR codes imprimés plus haut — le premier qui le lit peut se l'approprier. Quant à une vraie application mobile, qui détiendrait elle-même les clés, c'est un chantier pour plus tard, et sans doute pour quelqu'un d'autre. Ceci est la version qui ne vous demande de faire confiance à personne.

Il y a une seconde façon d'utiliser le téléphone, sans aucun bot : comme un endroit où garder des billets sous forme d'images. Un billet est une clé, et le code QR de cette clé est l'argent lui-même ; un téléphone qui contient quelques images de ce genre est un portefeuille avec quelques billets dedans. Traitez-les exactement comme des espèces : quiconque en voit, en copie ou en scanne une peut s'approprier le billet en y posant sa propre serrure, si bien qu'une capture d'écran dans un album partagé est un billet oublié sur une table. Marigold n'a pas d'application téléphone autonome, et c'est voulu. Le portefeuille complet tourne sur un ordinateur que vous contrôlez, et le téléphone en est soit la télécommande, soit une poche à billets.

---

## Ce qu'un observateur peut voir, et ce qu'il ne peut pas voir

Marigold n'est pas une cryptomonnaie anonyme, et ce document ne prétendra pas le contraire. Voici exactement ce que peut déduire quelqu'un qui observe le système :

**Il peut voir :** chaque billet, chaque valeur, chaque opération, l'offre totale et l'état courant complet du système. Tout est visible de tous. Rien n'est jamais chiffré.

**Il peut voir :** l'enchaînement des opérations — quel billet a donné quel nouveau billet, à quel moment les divisions et les fusions ont eu lieu, les rythmes dans le temps. Un analyste déterminé qui étudie les registres publics peut repérer des habitudes : des paiements rapprochés, des séquences « je divise puis je dépense », ou le rythme quotidien d'un commerçant qui reçoit des billets et les fusionne.

**Il peut voir :** qui a créé et qui a encaissé des billets, s'il parvient à relier une adresse blockchain ordinaire à une identité réelle. Les points où Marigold rejoint le monde blockchain traditionnel sont entièrement visibles.

**Il ne peut pas voir :** la moindre identité attachée à un paiement à l'intérieur du système. Il n'y a rien à voir, parce que la donnée n'a jamais été enregistrée.

Un billet se fond dans la masse de tous les billets de même valeur — un billet de 100 MAGLD ressemble à tous les autres billets de 100 MAGLD, et rien de plus. Les logiciels de portefeuille peuvent brouiller ces habitudes (varier légèrement le moment des opérations, les regrouper, alterner les coupures utilisées pour les frais), mais ce sont de bonnes pratiques, pas de la magie. Marigold ne promet rien de plus.

---

## Pourquoi c'est une monnaie honnête

Toutes les quelques secondes, chaque participant vérifie une règle unique :

**Tous les billets existants + toutes les pièces du côté traditionnel = total des pièces jamais minées.**

Si l'égalité ne tient pas, quelque chose ne va pas, et le réseau le voit immédiatement. L'offre est vérifiable à chaque instant — sans faire confiance à un auditeur, sans faire confiance à un système de preuves compliqué, mais avec le genre d'arithmétique que tout le monde sait faire.

Aucune valeur n'est jamais créée ni détruite à l'intérieur du système. Payer des frais retire un billet de l'ensemble, mais sa valeur est créditée au mineur et peut être recréée en un nouveau billet à tout moment. La règle de conservation est absolue et universelle.

---

## La chaîne : assez rapide pour ressembler à de l'argent liquide

La règle de règlement est simple : un billet est à vous quand votre changement de serrure est confirmé par le réseau. Autrement dit, **le temps de confirmation est le temps de la remise en main propre.** Sur Bitcoin, vous resteriez planté devant l'étal pendant dix minutes. Sur Ethereum, une douzaine de secondes. Sur Marigold, moins d'une seconde.

Marigold tourne sur une blockchain à preuve de travail qui produit environ 10 blocs par seconde, d'où une confirmation en moins d'une seconde. La technologie sous-jacente a été construite par le projet Kaspa — une blockchain rapide, fiable et éprouvée. Marigold ajoute par-dessus le système de billets et son économie, en laissant la couche de base intacte, parce qu'elle fait son travail remarquablement bien.

**Un mot sur l'énergie.** Ce que Marigold a à dire sur l'énergie, ce n'est pas qu'il en consomme peu — une chaîne à preuve de travail attire autant de minage que ses récompenses le justifient, et un Marigold à succès ne fera pas exception. C'est qu'aucun de ces watts ne sert à vous faire attendre. Une blockchain classique ne peut accepter qu'un bloc par tour ; les blocs minés en parallèle sont jetés, si bien que le réseau ne reste sûr qu'en restant lent. La chaîne sur laquelle tourne Marigold garde tous les blocs — ceux qui sont trouvés au même instant sont tissés ensemble dans le registre, et comptent tous — et c'est ainsi que le même budget de sécurité donne dix blocs par seconde et un règlement en moins d'une seconde, au lieu d'une file d'attente de dix minutes. Rapportée au paiement, la différence d'efficacité est énorme. Au total, c'est la même arithmétique honnête que partout ailleurs ici : de l'énergie dépensée à proportion de la valeur protégée.

---

## L'économie en un coup d'œil

- **Offre :** 210 000 000 MAGLD, plafond absolu. Aucun pré-minage, aucun fonds pour les développeurs, aucune attribution d'aucune sorte.
- **Lancement :** lancement équitable dès le premier jour. Logiciel disponible pour tous à l'avance. Tout le monde part à égalité.
- **Émission :** douce et progressive — la récompense de minage est divisée par deux tous les trois ans, sans chute brutale. Environ 21 % est miné la première année, ~90 % au bout de dix ans.
- **Unité de base :** 1 MAGLD = 100 000 000 pétales.
- **Coût d'un paiement :** 0,01 MAGLD — un centième de pièce — pour tout paiement courant, quel que soit le montant envoyé. Les frais sont un unique petit billet remis avec le paiement : déplacer un café ou une voiture coûte donc la même chose. Seules les opérations inhabituellement lourdes, qui regroupent des dizaines de billets d'un coup, montent à deux ou trois centièmes.
- **Frais :** tous les frais vont aux mineurs. Rien n'est brûlé, rien n'est détourné. Une économie d'argent liquide — où chaque paiement est une transaction sur la chaîne — produit un revenu de frais régulier que les chaînes de réserve de valeur ne peuvent pas égaler.

---

## Sécurité au lancement : des petites roues faites pour être enlevées

Une nouvelle chaîne à preuve de travail dispose de peu de puissance de minage, et une faible puissance de minage attire les attaques. Marigold démarre avec une protection temporaire et entièrement divulguée, appelée **ancres de finalité** : cinq garants publiquement identifiés co-signent périodiquement un bloc récent (il en faut au moins 3 sur 5), ce qui le rend permanent et irréversible. Personne ne peut défaire un bloc ancré.

Les garants ne peuvent pas censurer de transactions, créer des pièces, déplacer les fonds de qui que ce soit, ni produire des blocs. Leur seul pouvoir est d'empêcher qu'une transaction déjà faite soit défaite. S'ils se taisent, la chaîne continue normalement, comme un réseau à preuve de travail ordinaire — un peu moins protégée, mais pleinement opérationnelle.

Cette protection est conçue pour disparaître. Dès que la puissance de minage du réseau devient assez forte pour rendre une attaque hors de prix, les ancres passent d'obligatoires à consultatives, puis expirent tout à fait. Avant cela, le choix des garants suivants doit revenir à une gouvernance des détenteurs de billets — de sorte que même ce qui reste des petites roues passe des fondateurs à la communauté avant d'être retiré pour de bon.

Sur le réseau de test public, la protection tourne dès maintenant : cinq clés de garants, une ancre toutes les trente secondes environ, chaque nœud qui l'applique. Les garants du réseau de test sont des clés jetables faites pour la répétition ; les vrais sont choisis au lancement.

---

## Ce que Marigold est — et ce qu'il n'est pas

**Marigold est** de l'argent liquide numérique. Les billets sont des titres au porteur. Détenir la clé, c'est être propriétaire. Transmettre la clé, c'est régler. La chaîne dit tout de la valeur et rien des personnes — exactement comme un billet de banque.

**Marigold n'est pas** une cryptomonnaie anonyme. Rien n'est chiffré. Rien n'est caché. La distinction n'est pas un euphémisme : les cryptomonnaies anonymes dissimulent par la cryptographie des données enregistrées, tandis que Marigold n'enregistre aucune donnée à dissimuler. Ce qu'il partage avec l'argent liquide est précis et honnêtement délimité — le système dit tout de la *valeur*, par construction, et rien des *personnes*, par construction.

**Marigold est** cinq opérations, une règle de conservation, un état visible de tous, des signatures numériques ordinaires, une offre fixe que tout le monde vérifie à chaque instant, sur un réseau assez rapide pour que remettre un billet Marigold donne la même sensation que remettre un billet de banque.

Tout ce qu'il fait peut être vérifié par ceux à qui il est destiné.

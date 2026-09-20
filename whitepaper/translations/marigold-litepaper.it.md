# Marigold Litepaper

## Il problema: il denaro digitale ha dimenticato quello che il contante faceva bene

Tira fuori dal portafoglio una banconota da un dollaro. Guarda tre cose:

1. **Chi la tiene in mano, la possiede.** Non c'è nessun conto, nessun login, nessun intermediario. Il possesso è la proprietà.
2. **Non ha memoria.** La banconota non ricorda chi l'ha spesa per ultimo. Non esiste nessuna traccia che ti colleghi al caffè che hai preso stamattina.
3. **Chiunque può controllare che sia vera.** La metti in controluce, tocchi la carta. Non serve nessuna competenza particolare.

Nessuna forma diffusa di denaro digitale riproduce tutte e tre queste proprietà insieme.

Bitcoin ti dà la prima e la terza: avere le chiavi significa possedere le monete, e chiunque può verificare che il sistema sia onesto. Sulla seconda però fallisce, e di molto. Ogni Bitcoin mai minato si porta dietro tutta la sua storia, per sempre e in pubblico. Ogni indirizzo da cui hai mandato o su cui hai ricevuto finisce cucito dentro un grafo che chiunque può analizzare. Le tue monete ricordano tutto quello che ci hai fatto.

Le privacy coin come Monero e Zcash provano a rimediare cifrando la storia. A nascondere le cose riescono — ma è proprio la crittografia che nasconde a creare due problemi nuovi. Primo: quasi nessuno può verificare di persona che il sistema sia onesto. Ti fidi di esperti che non hai mai incontrato quando ti dicono che nella matematica non c'è nessun difetto, nessuna porta nascosta. Secondo: i regolatori danno un'occhiata a quella crittografia complicata e classificano la moneta di conseguenza — delisting, restrizioni e trattamento ostile arrivano non per quello che la moneta fa, ma per come lo fa.

Il risultato è un panorama in cui devi scegliere tra trasparenza senza privacy o privacy senza fiducia. Il contante offriva entrambe, senza sforzo. Il denaro digitale, chissà come, se n'è dimenticato.

---

## L'intuizione: non nascondere le tracce — non lasciarne

Immagina di essere alla bancarella di un mercato. Dai al venditore una banconota da cinque dollari. Il venditore la mette in cassa. Più tardi, lo stesso giorno, la spende dal panettiere. Il panettiere la dà di resto al cliente successivo.

Ora chiediti: chi può risalire da quella banconota a te? Nessuno. Non perché il suo viaggio fosse cifrato o nascosto — la banconota è stata in piena vista a ogni passaggio — ma perché la banconota stessa non ha mai registrato chi l'ha passata a chi. Il legame tra te e il venditore non è mai stato messo per iscritto. È esistito solo nell'istante del passaggio, e poi è sparito.

Questa è la differenza tra nascondere un'informazione e non raccoglierla mai. Le privacy coin nascondono. Marigold non raccoglie.

---

## Come funziona: banconote al posto dei conti

Quasi tutte le criptovalute funzionano come conti bancari. Hai un indirizzo (una specie di numero di conto) e le transazioni spostano valore tra indirizzi. Gli indirizzi sono pubblici e ogni movimento tra loro resta registrato per sempre. È il sogno di un contabile e un incubo per la privacy.

Marigold funziona come il contante. Non ci sono conti. Il sistema tiene invece un insieme di **banconote** — proprio come quelle di carta, ma digitali.

Ogni banconota è un semplice dato pubblico:

- Un **numero di serie** (come il numero di serie su una banconota da un dollaro)
- Un **valore** (0,01, 0,1, 1, 10, 100, 1.000, 10.000 o 100.000 MAGLD)
- Una **serratura** (una chiave pubblica — chi ha la chiave corrispondente può spenderla)

Tutto qui. Nessun nome del proprietario, nessun indirizzo, nessuna identità. La banconota non sa chi la tiene. Sa soltanto che esiste una chiave, e che chi riesce ad aprirla può spenderla.

### Fare un pagamento

Pagare qualcuno è come passargli una banconota, con un passaggio in più:

1. **Dai al destinatario la chiave** della banconota (con un codice QR, un messaggio o anche a voce — come faresti per condividere una foto).
2. **Il destinatario cambia subito la serratura** — registra una transazione che sostituisce la tua chiave con una chiave nuova di zecca che conosce solo lui. La conferma arriva in meno di un secondo.

Una volta confermato il cambio, la banconota è sua in modo irrevocabile. Tu non hai più una chiave che funziona. Il pagamento è chiuso.

In alternativa, se il destinatario può mandarti in anticipo la sua nuova chiave (per esempio con una richiesta di pagamento), puoi cambiare la serratura mettendo direttamente la sua. La tua chiave non viaggia da nessuna parte.

La terza via serve a pagare qualcuno che non c'è. Cambi la serratura mettendo una chiave che solo lui possiede e le dai una scadenza. Fino alla scadenza solo lui può prendere la banconota; dalla scadenza in poi solo tu puoi riprendertela. La incassa quando vuole; se non lo fa mai, il denaro torna tuo da solo. La scadenza la fa rispettare la rete, non il portafoglio di nessuno.

Tutte e tre finiscono allo stesso modo: la banconota sta sotto una chiave che solo il destinatario conosce.

### Dividere e unire

Le banconote hanno tagli fissi, esattamente come i soldi di carta. Se devi pagare 30 MAGLD e hai una banconota da 100 MAGLD, la **dividi** in dieci banconote da 10 MAGLD. Ne passi tre. Ne tieni sette. Dare il resto — come con il contante.

Nel senso opposto, dieci banconote da 10 MAGLD si possono **unire** in una sola da 100 MAGLD. I tagli vanno di dieci in dieci, così qualsiasi importo si paga con una manciata di banconote.

### Che cosa registra il sistema

Ogni operazione — creare, cambiare la serratura, dividere, unire, riscattare — è pubblica e in piena vista. Quello che nessuno può vedere è **chi** l'ha fatta, perché il sistema non ha nessuna nozione di «chi». Non c'è un campo mittente. Non c'è un campo destinatario. Non c'è una rubrica. Non c'è un conto.

Il sistema registra che una banconota ha cambiato mano. Non registra di chi fosse la mano. Una banconota con scadenza mostra anche le sue condizioni — la scadenza e la chiave a cui torna — finché la scadenza dura.

---

## Usare Marigold nel mondo reale

Dato che una banconota Marigold non è altro che una chiave, usarla nel mondo reale è semplice quanto usare il contante. Ecco come:

### Stampala, spendila

Ogni banconota nell'app del portafoglio può essere mostrata come **codice QR**. Quel codice QR puoi stamparlo su un foglio, piegarlo e metterlo nel portafoglio di pelle — accanto alle carte di credito e alla patente. Quel pezzo di carta *è* il denaro. Lo mostri a qualcuno, quello lo scansiona, cambia la serratura e la banconota è sua. Tu non hai più una chiave che funziona. Il foglio che hai in tasca ormai non vale niente — il valore è passato a lui.

Non è una metafora. Il codice QR stampato contiene la chiave vera. Perderlo è come perdere una banconota da cento dollari: chi la trova può spenderla. Tenerlo al sicuro è la stessa responsabilità che hai con i contanti. Non c'è un numero verde per il recupero, non c'è il pulsante «segnala il furto», non c'è una banca che annulla la transazione. Questo vuol dire essere al portatore, ed è tutto il punto.

### Regalala

Un biglietto di auguri con dentro un codice QR stampato è indistinguibile da un biglietto di auguri con dentro una banconota da cinquanta dollari. Chi lo riceve lo scansiona, cambia subito la serratura, e la banconota è sua in modo irrevocabile. Nessun conto da aprire, nessuna attesa. Apre la busta e ha i soldi in mano.

### Lasciala in eredità

Una busta sigillata in una cassetta di sicurezza, con dentro i codici QR stampati di alcune banconote, funziona esattamente come una busta piena di contanti. Chi la apre ha le chiavi. Chi ha le chiavi ha le banconote. Non serve un esecutore testamentario, non serve un tribunale, non serve il permesso di nessun terzo perché il valore passi di mano — anche se, come per il contante di carta, gli aspetti *legali* dell'eredità sono un'altra faccenda, che il sistema non affronta e non può affrontare. Il sistema garantisce solo questo: avere la chiave è avere la banconota.

### Paga alla bancarella

Stai comprando i pomodori. Il venditore mostra un codice QR: la sua richiesta di pagamento. Lo scansioni con l'app del portafoglio, scegli la banconota con cui vuoi pagare e il telefono cambia la serratura mettendo direttamente quella del venditore. Meno di un secondo dopo, la banconota è sua. Tu ti prendi i pomodori. Il venditore non ha visto il tuo nome, il tuo indirizzo o il tuo conto. Tu non hai visto i suoi. Il pagamento è chiuso, definitivo e dimenticabile — proprio come con il contante.

### Farsi pagare mentre non ci sei

Pubblica una sola chiave — sul biglietto da visita, in vetrina, nel tuo profilo — e chiunque può pagarti a qualsiasi ora. Ogni pagamento arriva sotto una chiave nuova che solo il tuo portafoglio sa ricavare da quella pubblicata, così la catena non mostra mai due pagamenti che arrivano nello stesso posto. Un pagamento a quella chiave può avere una scadenza, così chi manda sa che il denaro gli torna se non incassi mai. Il tuo portafoglio incassa ciò che aspetta la prossima volta che è acceso.

### Mettila in cassaforte

Hai paura degli hacker? Stampa le tue banconote come codici QR, mettile in una cassetta ignifuga e cancella l'app. Le banconote esistono sulla blockchain. Le chiavi esistono sulla carta. Non le tiene nessun dispositivo collegato a internet. Quando vuoi spendere, riporti il codice QR dentro un'app del portafoglio con una scansione, cambi subito la serratura (nel caso qualcuno abbia copiato il foglio mentre era custodito) e paghi normalmente.

### Spostala tra portafogli

Dato che le banconote sono chiavi indipendenti e non sono legate a nessuna frase di recupero né a nessun conto, puoi spostarne una da un'app all'altra quando vuoi. Esporti la chiave da un'app, la importi in un'altra. Nessuna transazione sulla catena, nessuna commissione, nessuna interazione con la rete. La tua banconota funziona allo stesso modo in ogni portafoglio che supporta Marigold — scegli l'app che ti piace, cambiala quando vuoi, i tuoi soldi ti seguono.

Il filo comune: una banconota Marigold è una chiave, e una chiave si può stampare, mandare per messaggio, piegare dentro una busta, attaccare al frigorifero o imparare a memoria. Al sistema non interessa come la chiave viaggia tra le persone, perché il sistema non sa che le persone esistono. Conosce solo chiavi e banconote — ed è per questo che ogni modo in cui puoi far girare una banconota di carta ha qui un equivalente digitale diretto.

---

## Il telefono è un telecomando, non un portafoglio

Tutto quello che fai dalla tua tastiera lo puoi fare dal telefono, in una chat di Telegram: controllare il saldo, pagare qualcuno, incassare una banconota, emettere una richiesta di pagamento, leggere lo storico, vedere se il tuo miner sta girando. Quello che cambia non è ciò che puoi fare. È dove stanno i soldi — e i soldi non sono sul telefono.

Il tuo portafoglio gira a casa, su qualunque cosa resti accesa: un portatile in un cassetto, un computerino accanto al router. Si tiene al passo con la catena, custodisce le tue banconote e risponde ai tuoi messaggi. Il telefono che hai in tasca non contiene chiavi e non conserva banconote. Non parla con nient'altro che Telegram. Se lo perdi, hai perso un telecomando.

Il bot è tuo, non nostro. Lo crei in Telegram in un paio di minuti, dai il token al tuo portafoglio e li accoppi con un codice. Da quel momento il tuo portafoglio risponde a quell'unico account Telegram e ignora tutti gli altri. Non c'è nessun server Marigold in mezzo, nessun account con noi, nessuna flotta di macchine gestita da noi da cui il tuo telefono debba dipendere. È il tuo portafoglio a chiamare Telegram; niente chiama dall'esterno, e non c'è niente da aprire sul router.

La spesa è protetta come con una carta bancaria: un PIN prima di qualsiasi cosa muova denaro, il blocco dopo tre tentativi sbagliati che solo la macchina di casa può togliere, e un limite giornaliero che imposti tu. Quell'account Telegram adesso può muovere denaro, quindi gli serve l'autenticazione a due fattori — il bot te lo dice la prima volta che ci parli.

I limiti, detti chiaramente. Quando la macchina di casa è spenta, il telefono non può fare proprio niente: niente saldo, niente pagamenti, niente incassi. Un codice di pagamento mandato in chat è valore al portatore finché è in viaggio, esattamente come i codici QR stampati di cui sopra: chi lo legge per primo se lo prende. E un'app per telefono vera e propria, che tenga lei le chiavi, è un lavoro per dopo e probabilmente per qualcun altro. Questa è la versione che non ti chiede di fidarti di nessuno.

---

## Che cosa vede e che cosa non vede un osservatore

Marigold non è una privacy coin, e questo litepaper non farà finta che lo sia. Ecco esattamente che cosa può ricavare chi osserva il sistema:

**Possono vedere:** ogni banconota, ogni valore, ogni operazione, l'offerta totale e lo stato attuale completo del sistema. Tutto è in piena vista. Non c'è mai niente di cifrato.

**Possono vedere:** la catena delle operazioni — quale banconota è diventata quale banconota nuova, quando sono avvenute divisioni e unioni, i ritmi temporali. Un analista ostinato che studia i registri pubblici può individuare schemi di comportamento: pagamenti rapidi uno dietro l'altro, sequenze dividi-e-spendi, o il ritmo quotidiano di un negoziante che incassa e unisce.

**Possono vedere:** chi ha creato e chi ha riscattato le banconote, se riescono a collegare un normale indirizzo blockchain a un'identità reale. I punti in cui Marigold si collega al mondo blockchain tradizionale sono del tutto visibili.

**Non possono vedere:** nessuna identità legata a un pagamento dentro il sistema. Non c'è niente da vedere, perché quel dato non è mai stato registrato.

Una banconota si confonde con tutte le altre dello stesso valore: una da 100 MAGLD è uguale a ogni altra da 100 MAGLD, e oltre quel gruppo non dice nulla. Il software del portafoglio può sfumare gli schemi di comportamento (variando un po' i tempi, raggruppando le operazioni, alternando i tagli usati per le commissioni), ma sono buone abitudini, non magia. Marigold non promette di più.

---

## Perché è denaro onesto

Ogni pochi secondi, ogni partecipante verifica una sola regola:

**Tutte le banconote esistenti + tutte le monete sul lato tradizionale = tutte le monete mai minate.**

Se il conto non torna, qualcosa non va e la rete se ne accorge subito. L'offerta è verificabile in ogni istante — non fidandosi di un revisore, non fidandosi di un sistema di prove complicato, ma con il tipo di aritmetica che sa fare chiunque.

Dentro il sistema non si crea né si distrugge mai valore. Pagare una commissione toglie una banconota dall'insieme, ma il suo valore viene accreditato al miner e può tornare a essere una banconota nuova in qualsiasi momento. La regola di conservazione è assoluta e vale per tutti.

---

## La catena: abbastanza veloce da sembrare contante

Il pagamento si chiude con una regola semplice: una banconota è tua quando il tuo cambio di serratura è confermato sulla rete. Vuol dire che **il tempo di conferma è il tempo del passaggio di mano.** Su Bitcoin resteresti fermo alla bancarella dieci minuti. Su Ethereum, circa dodici secondi. Su Marigold, meno di un secondo.

Marigold gira su una blockchain proof-of-work che produce circa 10 blocchi al secondo, con conferme sotto il secondo. La tecnologia di base è stata costruita dal progetto Kaspa — una blockchain veloce, affidabile e collaudata. Marigold aggiunge su quelle fondamenta il sistema delle banconote e la sua economia, e lascia intatto lo strato di base, perché il suo lavoro lo fa benissimo.

**Una parola sull'energia.** La storia energetica di Marigold non è che consuma poco: una catena proof-of-work attira tanto mining quanto valgono le sue ricompense, e se Marigold avrà successo non farà eccezione. La storia è che niente di quell'energia serve a farti aspettare. Una blockchain classica può accettare un solo blocco per turno; i blocchi minati in parallelo vengono buttati via, così la rete resta sicura solo restando lenta. La catena su cui gira Marigold tiene ogni blocco — i blocchi trovati nello stesso istante vengono intrecciati insieme nel registro, e contano tutti — ed è così che lo stesso budget di sicurezza dà dieci blocchi al secondo e pagamenti chiusi in meno di un secondo, invece di una coda di dieci minuti. Per ogni singolo pagamento la differenza di efficienza è enorme. Nel complesso è la stessa aritmetica onesta di tutto il resto: energia spesa in proporzione al valore che protegge.

---

## L'economia in breve

- **Offerta:** 210.000.000 MAGLD, tetto invalicabile. Nessuna moneta coniata in anticipo, nessun fondo per gli sviluppatori, nessuna assegnazione di alcun tipo.
- **Lancio:** equo dal primo giorno. Software a disposizione di tutti in anticipo. Si parte tutti alla pari.
- **Emissione:** regolare e graduale — la ricompensa di mining si dimezza ogni tre anni, senza cali improvvisi. Circa il 21% viene minato nel primo anno, il ~90% entro il decimo.
- **Unità base:** 1 MAGLD = 100.000.000 petali.
- **Costo di un pagamento:** 0,01 MAGLD — un centesimo di moneta — per qualsiasi pagamento quotidiano, qualunque sia la cifra che mandi. La commissione è una sola piccola banconota che passa insieme al pagamento, quindi spostare un caffè e spostare un'auto costa uguale. Solo le operazioni insolitamente grandi, che mettono insieme decine di banconote in un colpo, salgono a due o tre centesimi.
- **Commissioni:** vanno tutte ai miner. Niente viene bruciato, niente viene dirottato. Un'economia di contante — dove ogni pagamento è una transazione sulla catena — produce un flusso costante di commissioni che le catene pensate come riserva di valore non possono eguagliare.

---

## Sicurezza al lancio: rotelle che poi si tolgono

Una catena proof-of-work nuova ha poca potenza di mining, e poca potenza di mining invita gli attacchi. Marigold parte con una protezione temporanea e dichiarata per intero, chiamata **ancore di irreversibilità**: cinque garanti identificati pubblicamente firmano insieme, a intervalli regolari (ne bastano almeno 3 su 5), un blocco recente, e lo rendono permanente e irreversibile. Un blocco ancorato non lo può annullare nessuno.

I garanti non possono censurare transazioni, creare monete, spostare i fondi di qualcuno o produrre blocchi. Il loro unico potere è impedire che si annullino transazioni già concluse. Se tacciono, la catena va avanti normalmente come una rete proof-of-work qualsiasi — un po' meno protetta, ma perfettamente funzionante.

La protezione è fatta per andare in pensione. Quando la potenza di mining della rete sarà cresciuta abbastanza da rendere un attacco proibitivamente caro, le ancore passeranno da obbligatorie a consultive, e poi scadranno del tutto. Prima che accada, la scelta dei garanti successivi dovrebbe passare al voto di chi detiene le banconote — così anche quel che resta delle rotelle passa dai fondatori alla comunità, prima di essere tolto del tutto.

Sulla rete di prova pubblica la protezione è in funzione adesso: cinque chiavi di fiduciari, un'ancora ogni trenta secondi circa, ogni nodo che la fa rispettare. I fiduciari della rete di prova sono chiavi usa e getta fatte per la prova generale; quelli veri si scelgono al lancio.

---

## Che cos'è Marigold — e che cosa non è

**Marigold è** contante digitale. Le banconote sono strumenti al portatore. Avere la chiave è essere proprietario. Passare la chiave è chiudere il pagamento. La catena è completa sul valore e muta sulle persone — esattamente come una banconota da un dollaro.

**Marigold non è** una privacy coin. Niente è cifrato. Niente è nascosto. La distinzione non è un eufemismo: le privacy coin nascondono con la crittografia dati che comunque registrano, e Marigold non registra nessun dato da nascondere. Quello che ha in comune con il contante è preciso e onestamente delimitato — il sistema è completo sul *valore* per scelta, e muto sulle *persone* per scelta.

**Marigold è** cinque operazioni, una regola di conservazione, uno stato in piena vista, normali firme digitali, un'offerta fissa che tutti controllano in ogni momento, su una rete abbastanza veloce che passare una banconota è come passarne una di carta.

Tutto quello che fa può essere controllato dalle persone per cui è fatto.

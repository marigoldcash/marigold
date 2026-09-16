# Marigold Litepaper

## Das Problem: Digitales Geld hat vergessen, was Bargeld richtig macht

Nimm einen Geldschein aus deiner Brieftasche. Drei Dinge fallen auf:

1. **Wer ihn hat, dem gehört er.** Kein Konto, kein Login, kein Vermittler. Besitz ist Eigentum.
2. **Er hat kein Gedächtnis.** Der Schein weiß nicht, wer ihn zuletzt ausgegeben hat. Nichts verbindet dich mit dem Kaffee von heute Morgen.
3. **Jeder kann prüfen, ob er echt ist.** Gegen das Licht halten, das Papier fühlen. Dafür braucht es kein Fachwissen.

Kein verbreitetes digitales Geld bringt diese drei Eigenschaften zugleich zustande.

Bitcoin liefert die erste und die dritte: Wer die Schlüssel hat, dem gehören die Coins, und jeder kann nachprüfen, dass das System ehrlich ist. Bei der zweiten versagt Bitcoin gründlich. Jeder je geschürfte Bitcoin trägt seine vollständige Geschichte dauerhaft und öffentlich mit sich. Jede Adresse, von der du je gesendet oder auf der du je empfangen hast, hängt in einem Graphen, den jeder auswerten kann. Deine Coins erinnern sich an alles, was du je mit ihnen gemacht hast.

Privacy-Coins wie Monero und Zcash versuchen das zu lösen, indem sie die Geschichte verschlüsseln. Das Verstecken gelingt ihnen — aber genau die Kryptografie, die versteckt, schafft zwei neue Probleme. Erstens kann kaum jemand selbst nachprüfen, dass das System ehrlich ist. Du vertraust Fachleuten, die du nie getroffen hast, dass in der Mathematik kein Fehler steckt und keine Hintertür. Zweitens werfen Aufsichtsbehörden einen Blick auf die komplizierte Kryptografie und stufen die Währung entsprechend ein — Delistings, Auflagen und eine feindselige Behandlung folgen nicht daraus, was die Währung tut, sondern daraus, wie sie es tut.

Am Ende hast du die Wahl zwischen Transparenz ohne Privatsphäre und Privatsphäre ohne Vertrauen. Bargeld bot beides, ganz mühelos. Digitales Geld hat irgendwie vergessen, wie das geht.

---

## Die Einsicht: Die Spur nicht verwischen — gar keine hinterlassen

Stell dir vor, du stehst an einem Marktstand. Du gibst dem Händler einen Fünf-Euro-Schein. Der Händler legt ihn in die Kasse. Später am Tag gibt er genau diesen Schein beim Bäcker aus. Der Bäcker gibt ihn als Wechselgeld an den nächsten Kunden weiter.

Jetzt die Frage: Wer kann diesen Schein zu dir zurückverfolgen? Niemand. Nicht, weil sein Weg verschlüsselt oder versteckt gewesen wäre — der Schein lag bei jedem Schritt offen sichtbar da —, sondern weil der Schein selbst nie festgehalten hat, wer ihn wem gereicht hat. Die Verbindung zwischen dir und dem Händler wurde nie aufgeschrieben. Sie bestand nur im Moment der Übergabe und war dann weg.

Das ist der Unterschied zwischen Daten verstecken und Daten gar nicht erst sammeln. Privacy-Coins verstecken. Marigold sammelt nicht.

---

## So funktioniert es: Scheine statt Konten

Die meisten Kryptowährungen funktionieren wie Bankkonten. Du hast eine Adresse (so etwas wie eine Kontonummer), und Transaktionen schieben Werte zwischen Adressen hin und her. Die Adressen sind öffentlich, und jede Bewegung zwischen ihnen wird für immer festgehalten. Ein Traum für Buchhalter und ein Albtraum für die Privatsphäre.

Marigold funktioniert wie Bargeld. Es gibt keine Konten. Stattdessen verwaltet das System einen Bestand an **Scheinen** — denk an sie wie an digitale Geldscheine.

Jeder Schein ist ein schlichter öffentlicher Eintrag:

- Eine **Seriennummer** (wie die Seriennummer auf einem Geldschein)
- Ein **Wert** (0,01, 0,1, 1, 10, 100, 1.000, 10.000 oder 100.000 MAGLD)
- Ein **Schloss** (ein öffentlicher Schlüssel — wer den passenden Schlüssel hat, kann den Schein ausgeben)

Das ist alles. Kein Name eines Eigentümers, keine Adresse, keine Identität. Der Schein weiß nicht, wer ihn hat. Er weiß nur, dass es einen Schlüssel gibt, und dass ihn ausgeben kann, wer das Schloss öffnet.

### Bezahlen

Jemanden zu bezahlen ist wie einen Schein zu übergeben, nur mit einem Schritt mehr:

1. **Du gibst dem Empfänger den Schlüssel** zum Schein (per QR-Code, per Nachricht oder sogar mündlich — so, wie du ein Foto weitergeben würdest).
2. **Der Empfänger tauscht sofort das Schloss aus** — er schreibt eine Transaktion, die deinen Schlüssel durch einen ganz neuen ersetzt, den nur er kennt. Das ist in unter einer Sekunde bestätigt.

Sobald der Tausch bestätigt ist, gehört der Schein unwiderruflich ihm. Dein Schlüssel passt nicht mehr. Die Zahlung ist abgeschlossen.

Umgekehrt geht es auch: Wenn der Empfänger dir seinen neuen Schlüssel vorab schicken kann, etwa mit einer Zahlungsaufforderung, tauschst du das Schloss gleich gegen seinen Schlüssel aus. Dein Schlüssel verlässt dein Gerät dann nie. Beide Wege führen zum selben Ergebnis: Der Schein wandert von einem Schlüssel zum nächsten.

### Teilen und Zusammenlegen

Scheine gibt es in festen Werten, genau wie beim Bargeld. Wenn du 30 MAGLD zahlen willst und einen 100-MAGLD-Schein hast, **teilst** du ihn in zehn 10-MAGLD-Scheine. Drei davon gibst du weiter. Sieben behältst du. Wechselgeld — wie beim Bargeld.

Andersherum lassen sich zehn 10-MAGLD-Scheine zu einem 100-MAGLD-Schein **zusammenlegen**. Die Werte gehen in Zehnerschritten, deshalb ist jeder Betrag mit einer kleinen Handvoll Scheine zahlbar.

### Was das System aufzeichnet

Jede Operation — erzeugen, Schloss austauschen, teilen, zusammenlegen, einlösen — ist öffentlich und offen sichtbar. Was niemand sieht, ist, **wer** es war, denn das System kennt kein „Wer“. Es gibt kein Absenderfeld. Kein Empfängerfeld. Kein Adressbuch. Kein Konto.

Das System hält fest, dass ein Schein den Besitzer gewechselt hat. Wer die Besitzer sind, hält es nicht fest.

---

## Marigold im Alltag

Weil ein Marigold-Schein nichts weiter ist als ein Schlüssel, ist der Umgang damit im Alltag so einfach wie der mit Bargeld. So geht es:

### Ausdrucken und ausgeben

Jeder Schein in deiner Wallet-App lässt sich als **QR-Code** anzeigen. Du kannst diesen QR-Code auf ein Blatt Papier drucken, es zusammenfalten und in deine Brieftasche stecken — direkt neben die Kreditkarten und den Führerschein. Dieses Stück Papier *ist* das Geld. Zeig es jemandem, er scannt es, tauscht das Schloss aus, und der Schein gehört ihm. Dein Schlüssel passt nicht mehr. Das Papier in deiner Tasche ist jetzt wertlos — der Wert ist zu ihm gewandert.

Das ist kein Bild. Im gedruckten QR-Code steckt der echte Schlüssel. Ihn zu verlieren ist, als würdest du einen Hundert-Euro-Schein verlieren: Wer ihn findet, kann ihn ausgeben. Ihn sicher zu verwahren ist dieselbe Verantwortung wie bei Bargeld. Es gibt keine Hotline, keinen Knopf „als gestohlen melden“, keine Bank, die die Zahlung zurückholt. Genau das heißt Inhaberpapier, und darum geht es hier.

### Verschenken

Eine Geburtstagskarte mit einem gedruckten QR-Code darin ist nicht zu unterscheiden von einer Geburtstagskarte mit einem Fünfzig-Euro-Schein darin. Der Beschenkte scannt ihn, tauscht sofort das Schloss aus, und der Schein gehört unwiderruflich ihm. Kein Konto einrichten, keine Wartezeit. Er macht den Umschlag auf und hat das Geld in der Hand.

### Vererben

Ein versiegelter Umschlag im Bankschließfach, mit gedruckten QR-Codes für mehrere Scheine, wirkt genau wie ein Umschlag voll Bargeld. Wer ihn öffnet, hat die Schlüssel. Wer die Schlüssel hat, hat die Scheine. Es braucht keinen Testamentsvollstrecker, kein Nachlassgericht, keine Erlaubnis von dritter Seite, damit der Wert übergeht — wobei, wie bei Bargeld, die *rechtliche* Seite des Erbens eine eigene Sache ist, die das System nicht regelt und nicht regeln kann. Das System garantiert nur eines: Wer den Schlüssel hat, hat den Schein.

### Am Marktstand bezahlen

Du kaufst Tomaten. Der Händler zeigt einen QR-Code — seine Zahlungsaufforderung. Du scannst ihn mit deiner Wallet-App, wählst den Schein aus, mit dem du zahlen willst, und dein Handy tauscht das Schloss direkt gegen den Schlüssel des Händlers aus. Keine Sekunde später gehört der Schein ihm. Du steckst die Tomaten ein. Der Händler hat deinen Namen nie gesehen, deine Adresse nicht und dein Konto auch nicht. Du hast von ihm genauso wenig gesehen. Die Zahlung ist erledigt, endgültig und vergessen — wie beim Bargeld.

### Kalt lagern

Angst vor Hackern? Druck deine Scheine als QR-Codes aus, leg sie in eine feuerfeste Kassette und lösch die App. Die Scheine liegen in der Blockchain. Die Schlüssel liegen auf Papier. Kein Gerät mit Internetanschluss hat sie. Wenn du zahlen willst, scannst du den QR-Code zurück in eine Wallet-App, tauschst sofort das Schloss aus (falls jemand das Papier während der Lagerung abfotografiert hat) und zahlst ganz normal.

### Zwischen Wallets umziehen

Weil Scheine eigenständige Schlüssel sind und an keine Seed-Phrase und kein Konto gebunden, kannst du einen Schein jederzeit von einer Wallet-App in eine andere holen. Schlüssel in der einen App exportieren, in der anderen importieren. Keine Transaktion auf der Chain, keine Gebühr, überhaupt kein Kontakt mit dem Netzwerk. Dein Schein funktioniert in jeder Wallet gleich, die Marigold unterstützt — nimm die App, die dir gefällt, wechsle, wann du willst, dein Geld kommt mit.

Der rote Faden: Ein Marigold-Schein ist ein Schlüssel, und ein Schlüssel lässt sich drucken, verschicken, in einen Umschlag falten, an den Kühlschrank heften oder auswendig lernen. Dem System ist egal, wie der Schlüssel von Mensch zu Mensch kommt, denn das System weiß nicht, dass es Menschen gibt. Es kennt nur Schlüssel und Scheine — und deshalb hat jeder Weg, auf dem man eine Banknote weitergeben kann, hier eine direkte digitale Entsprechung.

---

## Dein Handy ist eine Fernbedienung, keine Geldbörse

Alles, was du an deiner eigenen Tastatur tun kannst, geht auch vom Handy aus, in einem Telegram-Chat: Guthaben abfragen, jemanden bezahlen, einen Schein annehmen, eine Zahlungsaufforderung stellen, deinen Verlauf lesen, nachsehen, ob dein Miner läuft. Was sich ändert, ist nicht, was du tun kannst. Es ist, wo das Geld liegt — und das Geld liegt nicht auf dem Handy.

Deine Wallet läuft zu Hause, auf dem, was ohnehin an bleibt: ein Laptop in der Schublade, ein kleiner Rechner neben dem Router. Sie hält sich mit der Chain auf Stand, sie verwahrt deine Scheine, und sie beantwortet deine Nachrichten. Das Handy in deiner Tasche hat keine Schlüssel und speichert keine Scheine. Es redet mit nichts außer Telegram. Verlierst du es, hast du eine Fernbedienung verloren.

Der Bot gehört dir, nicht uns. Du legst ihn in Telegram in etwa zwei Minuten an, gibst deiner Wallet das Token und verbindest beide mit einem Code. Von da an antwortet deine Wallet genau diesem einen Telegram-Konto und ignoriert jedes andere. Kein Marigold-Server dazwischen, kein Konto bei uns, keine Maschinenflotte von uns, auf die dein Handy angewiesen wäre. Deine Wallet ruft bei Telegram an; bei ihr ruft niemand an, und an deinem Router musst du nichts öffnen.

Das Ausgeben ist abgesichert wie bei einer Bankkarte: eine PIN vor allem, was Geld bewegt, eine Sperre nach drei Fehlversuchen, die nur der Rechner zu Hause aufheben kann, und ein Tageslimit, das du selbst festlegst. Dieses Telegram-Konto kann jetzt Geld bewegen, also braucht es Zwei-Faktor-Authentifizierung — der Bot sagt dir das beim ersten Mal, wenn du mit ihm sprichst.

Die Grenzen, ganz offen. Wenn der Rechner zu Hause aus ist, kann das Handy gar nichts: kein Guthaben, kein Bezahlen, kein Annehmen. Ein Zahlungscode, der durch einen Chat geht, ist unterwegs ein Inhaberpapier, genau wie die gedruckten QR-Codes weiter oben — wer ihn zuerst liest, kann ihn einlösen. Und eine richtige Handy-App, die selbst Schlüssel verwahrt, ist eine Aufgabe für später und wahrscheinlich für jemand anderen. Das hier ist die Version, die von dir verlangt, niemandem zu vertrauen.

---

## Was ein Beobachter sehen kann — und was nicht

Marigold ist kein Privacy-Coin, und dieses Litepaper tut auch nicht so. Hier steht genau, was jemand herausfinden kann, der das System beobachtet:

**Sichtbar ist:** Jeder Schein, jeder Wert, jede Operation, die Gesamtmenge und der vollständige aktuelle Zustand des Systems. Alles liegt offen da. Nichts ist verschlüsselt, zu keinem Zeitpunkt.

**Sichtbar ist:** Die Kette der Operationen — welcher Schein zu welchem neuen Schein wurde, wann geteilt und zusammengelegt wurde, zeitliche Muster. Wer die öffentlichen Aufzeichnungen hartnäckig genug durchgeht, erkennt Verhaltensmuster: schnelle Zahlungen hintereinander, erst teilen und dann ausgeben, oder den täglichen Rhythmus eines Händlers aus Annehmen und Zusammenlegen.

**Sichtbar ist:** Wer Scheine erzeugt und eingelöst hat — vorausgesetzt, es gelingt, eine gewöhnliche Blockchain-Adresse einer echten Person zuzuordnen. Die Stellen, an denen Marigold an die herkömmliche Blockchain-Welt andockt, liegen vollständig offen.

**Nicht sichtbar ist:** Irgendeine Identität hinter einer Zahlung innerhalb des Systems. Da ist nichts zu sehen, weil die Daten nie aufgezeichnet wurden.

Ein Schein geht in allen anderen Scheinen desselben Werts auf — ein 100-MAGLD-Schein sieht aus wie jeder andere 100-MAGLD-Schein, und weiter als bis zu dieser Gruppe reicht die Anonymität nicht. Wallet-Software kann Verhaltensmuster verwischen: Zeitpunkte leicht streuen, Operationen bündeln, unterschiedliche Scheingrößen für Gebühren nehmen. Aber das sind gute Gewohnheiten, keine Zauberei. Mehr behauptet Marigold nicht.

---

## Warum es ehrliches Geld ist

Alle paar Sekunden prüft jeder Teilnehmer eine einzige Regel:

**Alle vorhandenen Scheine + alle Coins auf der herkömmlichen Seite = alle je geschürften Coins.**

Stimmt das nicht, ist etwas faul, und das Netzwerk sieht es sofort. Die Geldmenge ist in jedem Augenblick nachprüfbar — nicht, weil man einem Prüfer glaubt, nicht, weil man einem komplizierten Beweissystem glaubt, sondern mit der Sorte Rechnen, die jeder beherrscht.

Innerhalb des Systems entsteht nie Wert und geht nie Wert verloren. Eine Gebühr zu zahlen nimmt einen Schein aus dem Bestand, aber sein Wert wird dem Miner gutgeschrieben und kann jederzeit wieder als neuer Schein erzeugt werden. Die Erhaltungsregel gilt absolut und ausnahmslos.

---

## Die Chain: Schnell genug, um sich wie Bargeld anzufühlen

Die Regel für den Abschluss ist einfach: Ein Schein gehört dir, sobald dein Schlosstausch im Netzwerk bestätigt ist. Das heißt: **Bestätigungszeit ist Übergabezeit.** Bei Bitcoin ständest du zehn Minuten am Marktstand. Bei Ethereum etwa zwölf Sekunden. Bei Marigold unter einer Sekunde.

Marigold läuft auf einer Proof-of-Work-Blockchain, die ungefähr 10 Blöcke pro Sekunde erzeugt und damit in unter einer Sekunde bestätigt. Die Technik darunter stammt vom Kaspa-Projekt — eine schnelle, zuverlässige, gut erprobte Blockchain. Marigold setzt das Scheinsystem und seine Ökonomie obendrauf und lässt die Basisschicht unangetastet, weil sie ihre Arbeit außergewöhnlich gut macht.

**Ein Wort zum Energieverbrauch.** Marigold verbraucht nicht wenig Energie, und das behauptet hier auch niemand — eine Proof-of-Work-Chain zieht so viel Mining an, wie ihre Belohnungen wert sind, und ein erfolgreiches Marigold wird da keine Ausnahme sein. Der Punkt ist ein anderer: Nichts davon wird fürs Warten ausgegeben. Eine klassische Blockchain kann pro Runde nur einen Block annehmen; parallel geschürfte Blöcke landen im Müll, und das Netzwerk bleibt nur sicher, indem es langsam bleibt. Die Chain, auf der Marigold läuft, behält jeden Block — Blöcke, die im selben Moment gefunden werden, werden gemeinsam ins Hauptbuch eingewoben und zählen alle —, und deshalb liefert derselbe Sicherheitsaufwand zehn Blöcke pro Sekunde und einen Abschluss in unter einer Sekunde statt einer Warteschlange von zehn Minuten. Pro Zahlung ist das ein gewaltiger Unterschied in der Effizienz. Insgesamt ist es dieselbe ehrliche Rechnung wie alles andere hier: Energie im Verhältnis zu dem Wert, den sie schützt.

---

## Die Ökonomie auf einen Blick

- **Menge:** 210.000.000 MAGLD, harte Obergrenze. Nichts vorab geschürft, kein Entwicklerfonds, keine Zuteilung irgendeiner Art.
- **Start:** Fairer Start ab Tag eins. Die Software steht vorab allen zur Verfügung. Alle fangen unter gleichen Bedingungen an.
- **Ausgabe:** Gleichmäßig und allmählich — die Mining-Belohnung halbiert sich alle drei Jahre, ohne plötzliche Einbrüche. Etwa 21 % werden im ersten Jahr geschürft, rund 90 % bis zum zehnten.
- **Kleinste Einheit:** 1 MAGLD = 100.000.000 Blütenblätter.
- **Was eine Zahlung kostet:** 0,01 MAGLD — ein Hundertstel Coin — für jede alltägliche Zahlung, egal wie viel du schickst. Die Gebühr ist ein einzelner kleiner Schein, den du mit der Zahlung übergibst, deshalb kostet ein Kaffee dasselbe wie ein Auto. Nur ungewöhnlich große Operationen, die Dutzende Scheine auf einmal bündeln, steigen auf zwei oder drei Hundertstel.
- **Gebühren:** Alle Gebühren gehen an die Miner. Nichts wird verbrannt, nichts umgeleitet. Eine Bargeldwirtschaft — in der jede Zahlung eine Transaktion auf der Chain ist — bringt stetige Gebühreneinnahmen, mit denen Chains als reiner Wertspeicher nicht mithalten können.

---

## Sicherheit beim Start: Stützräder, die wieder abkommen

Eine neue Proof-of-Work-Chain hat wenig Rechenleistung hinter sich, und wenig Rechenleistung lädt zu Angriffen ein. Marigold startet mit einer befristeten, vollständig offengelegten Absicherung namens **Finalitätsanker**: Fünf öffentlich benannte Treuhänder unterschreiben in regelmäßigen Abständen gemeinsam einen jungen Block — mindestens 3 von 5 müssen mitmachen — und machen ihn damit endgültig und unumkehrbar. Einen verankerten Block kann niemand mehr rückgängig machen.

Die Treuhänder können keine Transaktionen zensieren, keine Coins erzeugen, niemandes Geld bewegen und keine Blöcke produzieren. Ihre einzige Macht ist, das Rückgängigmachen abgeschlossener Transaktionen zu verhindern. Wenn sie verstummen, läuft die Chain ganz normal als gewöhnliches Proof-of-Work-Netzwerk weiter — etwas weniger geschützt, aber voll funktionsfähig.

Die Absicherung ist darauf angelegt, wieder zu verschwinden. Sobald die Rechenleistung im Netzwerk stark genug ist, dass Angriffe unbezahlbar werden, werden die Anker von verbindlich zu empfehlend und treten am Ende ganz außer Kraft. Vorher soll die Auswahl der nachfolgenden Treuhänder an die Mitbestimmung der Scheininhaber übergehen — selbst der Rest der Stützräder wechselt also von den Gründern zur Gemeinschaft, bevor sie ganz abgenommen werden.

---

## Was Marigold ist — und was nicht

**Marigold ist** digitales Bargeld. Scheine sind Inhaberpapiere. Wer den Schlüssel hat, dem gehört der Schein. Wer den Schlüssel weitergibt, hat bezahlt. Die Chain führt lückenlos Buch über Werte und schweigt über Menschen — genau wie ein Geldschein.

**Marigold ist kein** Privacy-Coin. Nichts ist verschlüsselt. Nichts ist versteckt. Der Unterschied ist keine Beschönigung: Privacy-Coins verbergen aufgezeichnete Daten mit Kryptografie, und Marigold zeichnet keine Daten auf, die man verbergen müsste. Was es mit Bargeld teilt, ist genau benannt und ehrlich begrenzt — das System führt von Grund auf lückenlos Buch über *Werte* und schweigt von Grund auf über *Menschen*.

**Marigold ist** fünf Operationen, eine Erhaltungsregel, ein offen sichtbarer Zustand, gewöhnliche digitale Signaturen, eine feste Geldmenge, die jeder jederzeit nachrechnet, auf einem Netzwerk, das schnell genug ist, damit sich das Weitergeben eines Scheins anfühlt wie das Weitergeben einer Banknote.

Alles, was es tut, können die Leute nachprüfen, für die es gemacht ist.

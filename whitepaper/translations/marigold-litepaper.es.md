# Litepaper de Marigold

## El problema: el dinero digital olvidó lo que el efectivo hacía bien

Saca un billete de la cartera. Fíjate en tres cosas:

1. **Quien lo tiene, es su dueño.** No hay cuenta, ni inicio de sesión, ni intermediario. La posesión es la propiedad.
2. **No guarda memoria.** El billete no recuerda quién lo gastó la última vez. No existe ningún registro que te vincule con el café que compraste esta mañana.
3. **Cualquiera puede comprobar que es auténtico.** Lo miras al trasluz, tocas el papel. No hace falta ser experto.

Ningún dinero digital de uso corriente reúne las tres propiedades a la vez.

Bitcoin te da la primera y la tercera: tener tus llaves significa ser dueño de tus monedas, y cualquiera puede verificar que el sistema es honesto. Pero falla estrepitosamente en la segunda. Cada bitcoin que se ha minado arrastra su historial completo, de forma permanente y pública. Cada dirección desde la que has enviado algo o en la que has recibido algo queda cosida a las demás en un grafo que cualquiera puede analizar. Tus monedas recuerdan todo lo que hiciste con ellas.

Las monedas de privacidad como Monero y Zcash intentan arreglarlo cifrando el historial. Y lo consiguen: esconden cosas. Pero la misma criptografía que las esconde crea dos problemas nuevos. El primero: casi nadie puede verificar por sí mismo que el sistema es honesto. Estás confiando en expertos que no conoces, en que las matemáticas no esconden ningún fallo ni ninguna puerta trasera. El segundo: los reguladores echan un vistazo a esa criptografía compleja y clasifican la moneda en consecuencia — la exclusión de las casas de cambio, las restricciones y el trato hostil no llegan por lo que la moneda hace, sino por cómo lo hace.

El resultado es un panorama en el que eliges entre transparencia sin privacidad o privacidad sin confianza. El efectivo ofrecía las dos cosas, sin esfuerzo. El dinero digital, de algún modo, lo olvidó.

---

## La idea: no ocultes el rastro — no dejes ninguno

Imagina que estás en un puesto del mercado. Le das un billete de cinco dólares al vendedor. El vendedor lo mete en la caja. Más tarde, ese mismo día, el vendedor se gasta ese mismo billete en la panadería. La panadería se lo da de cambio al siguiente cliente.

Ahora pregunta: ¿quién puede rastrear ese billete hasta ti? Nadie. No porque el recorrido del billete estuviera cifrado u oculto — el billete estuvo a plena vista en cada paso del camino — sino porque el billete nunca registró quién se lo entregó a quién. La conexión entre tú y el vendedor no se escribió en ninguna parte. Existió solo en el momento de la entrega y después desapareció.

Esta es la diferencia entre ocultar información y no recogerla nunca. Las monedas de privacidad ocultan. Marigold no recoge.

---

## Cómo funciona: billetes en lugar de cuentas

La mayoría de las criptomonedas funcionan como cuentas bancarias. Tienes una dirección (parecida a un número de cuenta) y las transacciones mueven valor entre direcciones. Las direcciones son públicas y cada movimiento entre ellas queda registrado para siempre. Es el sueño de un contable y la pesadilla de la privacidad.

Marigold funciona como el efectivo. No hay cuentas. En su lugar, el sistema mantiene un conjunto de **billetes** — piensa en ellos como billetes de banco digitales.

Cada billete es un registro público sencillo:

- Un **número de serie** (como el número de serie de un billete de papel)
- Un **valor** (0,01, 0,1, 1, 10, 100, 1.000, 10.000 o 100.000 MAGLD)
- Una **cerradura** (una llave pública: quien tenga la llave que encaja puede gastarlo)

Eso es todo. Sin nombre de dueño, sin dirección, sin identidad. El billete no sabe quién lo tiene. Solo sabe que existe una llave, y que quien pueda abrir la cerradura puede gastarlo.

### Hacer un pago

Pagar a alguien es como entregar un billete, con un paso más:

1. **Le das al destinatario la llave** del billete (por código QR, por mensaje de texto o incluso de viva voz — igual que compartirías una foto).
2. **El destinatario cambia la cerradura de inmediato** — registra una transacción que sustituye tu llave por una llave nueva que solo él conoce. Se confirma en menos de un segundo.

Una vez confirmado ese cambio, el billete es suyo de forma irrevocable. Tú ya no tienes una llave que funcione. La liquidación está hecha.

Como alternativa, si el destinatario puede enviarte por adelantado su llave nueva (por ejemplo, con una solicitud de pago), puedes cambiar la cerradura directamente a su llave. Tu llave no viaja a ninguna parte. Los dos métodos producen el mismo resultado: el billete pasa de una llave a otra.

### Dividir y unir

Los billetes vienen en valores fijos, igual que el dinero de papel. Si necesitas pagar 30 MAGLD y tienes un billete de 100 MAGLD, lo **divides** en diez billetes de 10 MAGLD. Entregas tres. Te quedas siete. Dar cambio, igual que con el efectivo.

En sentido contrario, diez billetes de 10 MAGLD pueden **unirse** en uno solo de 100 MAGLD. Los valores se mueven en factores de diez, así que cualquier cantidad se paga con un puñado pequeño de billetes.

### Qué registra el sistema

Cada operación — crear, cambiar cerraduras, dividir, unir, canjear — es pública y está a plena vista. Lo que nadie puede ver es **quién** la hizo, porque el sistema no tiene ningún concepto de «quién». No hay campo de remitente. Ni campo de destinatario. Ni agenda de direcciones. Ni cuenta.

El sistema registra que un billete cambió de manos. No registra de qué manos.

---

## Usar Marigold en el mundo real

Como un billete Marigold no es más que una llave, usarlo en el mundo real es tan sencillo como usar efectivo. Así es como se hace:

### Imprímelo, gástalo

Cada billete de tu aplicación de monedero se puede mostrar como un **código QR**. Puedes imprimir ese código QR en un papel, doblarlo y meterlo en tu cartera de cuero, justo al lado de las tarjetas de crédito y el carné de conducir. Ese papel *es* el dinero. Se lo entregas a alguien, lo escanea, cambia la cerradura y el billete es suyo. Tú ya no tienes una llave que funcione. El papel que ahora lleva en el bolsillo no vale nada: el valor se ha movido.

Esto no es una metáfora. El código QR impreso contiene la llave de verdad. Perderlo es como perder un billete de cien dólares: quien lo encuentre puede gastarlo. Guardarlo a salvo es la misma responsabilidad que guardar efectivo a salvo. No hay teléfono de atención para recuperarlo, ni botón de «denunciar robo», ni banco que revierta la transacción. Eso es lo que significa ser un instrumento al portador, y en eso consiste todo.

### Regálalo

Una tarjeta de cumpleaños con un código QR impreso dentro no se distingue de una tarjeta de cumpleaños con un billete de cincuenta dólares dentro. Quien la recibe lo escanea, cambia la cerradura de inmediato y el billete es suyo de forma irrevocable. No hay cuenta que abrir ni plazo de espera. Abre el sobre y ya tiene el dinero en la mano.

### Déjalo en herencia

Un sobre cerrado en una caja de seguridad, con los códigos QR impresos de varios billetes, funciona exactamente igual que un sobre lleno de efectivo. Quien lo abre tiene las llaves. Quien tiene las llaves tiene los billetes. No hace falta albacea, ni juicio testamentario, ni permiso de ningún tercero para que el valor cambie de manos — aunque, igual que con el efectivo, los arreglos *legales* en torno a una herencia son un asunto aparte que el sistema no aborda ni puede abordar. El sistema garantiza solo que tener la llave es tener el billete.

### Paga en un puesto del mercado

Estás comprando tomates. El vendedor muestra un código QR: su solicitud de pago. Lo escaneas con tu aplicación de monedero, eliges el billete con el que quieres pagar y tu teléfono cambia la cerradura directamente a la llave del vendedor. Menos de un segundo después, el billete es suyo. Te guardas los tomates. El vendedor nunca vio tu nombre, ni tu dirección, ni tu cuenta. Tú tampoco viste los suyos. La transacción queda liquidada, es definitiva y se puede olvidar, igual que el efectivo.

### Guárdalo en frío

¿Te preocupan los hackers? Imprime tus billetes como códigos QR, mételos en una caja ignífuga y borra la aplicación. Los billetes existen en la blockchain. Las llaves existen en papel. Ningún aparato conectado a internet las guarda. Cuando quieras gastar, escanea el código QR para devolverlo a una aplicación de monedero, cambia la cerradura de inmediato (por si alguien copió el papel mientras estaba guardado) y opera con normalidad.

### Cámbialo de monedero

Como los billetes son llaves independientes y no están atados a ninguna frase semilla ni a ninguna cuenta, puedes llevar un billete de una aplicación de monedero a otra cuando quieras. Exporta la llave de una aplicación e impórtala en otra. No hace falta ninguna transacción en la cadena, no se paga ninguna comisión, no hay interacción alguna con la red. Tu billete funciona igual en cualquier monedero compatible con Marigold: elige la aplicación que te guste, cámbiala cuando quieras, tu dinero va contigo.

El hilo común: un billete Marigold es una llave, y una llave se puede imprimir, mandar por mensaje, doblar dentro de un sobre, pegar en la nevera o aprender de memoria. Al sistema le da igual cómo viaja la llave entre las personas, porque el sistema no sabe que las personas existen. Solo conoce llaves y billetes — y por eso cada forma de mover un billete de papel tiene aquí un equivalente digital directo.

---

## Tu teléfono es un control remoto, no un monedero

Todo lo que puedes hacer desde tu propio teclado lo puedes hacer desde el teléfono, en un chat de Telegram: consultar tu saldo, pagar a alguien, recibir un billete, emitir una solicitud de pago, leer tu historial, ver si tu minero está funcionando. Lo que cambia no es lo que puedes hacer. Es dónde está el dinero — y el dinero no está en el teléfono.

Tu monedero funciona en casa, en lo que sea que quede encendido: una computadora portátil en un cajón, una máquina pequeña junto al router. Se mantiene al día con la cadena, guarda tus billetes y responde a tus mensajes. El teléfono que llevas en el bolsillo no guarda llaves ni almacena billetes. No habla con nada más que con Telegram. Si lo pierdes, has perdido un control remoto.

El bot es tuyo, no nuestro. Lo creas en Telegram en unos dos minutos, le entregas el token a tu monedero y emparejas los dos con un código. A partir de ahí tu monedero responde a esa única cuenta de Telegram e ignora todas las demás. No hay ningún servidor de Marigold en medio, ninguna cuenta con nosotros, ninguna flota de máquinas nuestras de la que tu teléfono dependa. Tu monedero llama hacia fuera, a Telegram; nada llama hacia dentro, y no hay nada que abrir en tu router.

El gasto está protegido como el de una tarjeta bancaria: un PIN antes de cualquier cosa que mueva dinero, un bloqueo tras tres intentos fallidos que solo la máquina de casa puede levantar, y un límite diario que fijas tú. Esa cuenta de Telegram ya puede mover dinero, así que necesita autenticación de dos factores: el bot te lo dice la primera vez que hablas con él.

Los límites, dichos claramente. Cuando la máquina de casa está apagada, el teléfono no puede hacer absolutamente nada: ni saldo, ni pagos, ni cobros. Un código de pago enviado por un chat es valor al portador mientras está en tránsito, exactamente igual que los códigos QR impresos de antes: quien lo lea primero se lo queda. Y una aplicación de teléfono en condiciones, una que guarde las llaves ella misma, es tarea para más adelante y probablemente para otra gente. Esta es la versión que no te pide confiar en nadie.

---

## Lo que un observador puede y no puede ver

Marigold no es una moneda de privacidad, y este litepaper no va a fingir que lo sea. Esto es exactamente lo que puede averiguar alguien que observe el sistema:

**Puede ver:** cada billete, cada valor, cada operación, el suministro total y el estado actual completo del sistema. Todo está a plena vista. Nada está cifrado, nunca.

**Puede ver:** la cadena de operaciones — qué billete se cambió por qué billete nuevo, cuándo hubo divisiones y uniones, los patrones de tiempos. Un analista decidido que estudie los registros públicos puede identificar patrones de comportamiento: pagos rápidos y seguidos, secuencias de dividir y gastar, o el ritmo diario de aceptar y unir de un comercio.

**Puede ver:** quién creó y canjeó billetes, si consigue vincular una dirección corriente de la blockchain con una identidad del mundo real. Los puntos donde Marigold se conecta con el mundo tradicional de la blockchain son completamente visibles.

**No puede ver:** ninguna identidad asociada a un pago dentro del sistema. No hay nada que ver, porque ese dato nunca se registró.

Un billete se confunde con todos los demás billetes del mismo valor: un billete de 100 MAGLD se parece a cualquier otro billete de 100 MAGLD y no dice nada más allá de ese grupo. El software de monedero puede difuminar los patrones de comportamiento (variando un poco los tiempos, agrupando operaciones, alternando qué tamaños de billete se usan para las comisiones), pero eso son buenos hábitos, no magia. Marigold no afirma nada más fuerte que eso.

---

## Por qué es dinero honesto

Cada pocos segundos, cada participante verifica una sola regla:

**Todos los billetes que existen + todas las monedas del lado tradicional = total de monedas minadas hasta hoy.**

Si esto no se cumple, algo va mal y la red lo ve de inmediato. El suministro es comprobable en todo momento — no porque confíes en un auditor, no porque confíes en un sistema de pruebas complicado, sino con la clase de aritmética que cualquiera sabe hacer.

Dentro del sistema nunca se crea ni se destruye valor. Pagar una comisión retira un billete del conjunto, pero su valor se acredita al minero y puede volver a crearse como billete nuevo en cualquier momento. La regla de conservación es absoluta y universal.

---

## La cadena: lo bastante rápida para sentirse como efectivo

La regla de liquidación es sencilla: un billete es tuyo cuando tu cambio de cerradura queda confirmado en la red. Eso significa que **el tiempo de confirmación es el tiempo de entrega.** En Bitcoin, estarías diez minutos de pie en el puesto del mercado. En Ethereum, unos doce segundos. En Marigold, menos de un segundo.

Marigold funciona sobre una blockchain de prueba de trabajo que produce unos 10 bloques por segundo, lo que da confirmaciones por debajo del segundo. La tecnología de base la construyó el proyecto Kaspa: una blockchain rápida, fiable y bien probada. Marigold añade encima el sistema de billetes y su economía, y deja intacta la capa base porque hace su trabajo excepcionalmente bien.

**Un apunte sobre la energía.** La historia energética de Marigold no es que consuma poco — una cadena de prueba de trabajo atrae tanta minería como valgan sus recompensas, y una Marigold exitosa no será una excepción. La historia es que nada de esa energía compra espera. Una blockchain clásica solo puede aceptar un bloque por ronda; los bloques minados en paralelo se tiran a la basura, así que la red solo se mantiene segura manteniéndose lenta. La cadena sobre la que corre Marigold conserva todos los bloques — los que se encuentran en el mismo instante se entretejen juntos en el libro de cuentas, y todos cuentan — y así el mismo presupuesto de seguridad entrega diez bloques por segundo y liquidación por debajo del segundo en lugar de una cola de diez minutos. Por pago, esa diferencia de eficiencia es enorme. En total, es la misma aritmética honesta que todo lo demás aquí: energía gastada en proporción al valor que se protege.

---

## La economía de un vistazo

- **Suministro:** 210.000.000 MAGLD, tope fijo. Sin preminado, sin fondo de desarrollo, sin asignación de ningún tipo.
- **Lanzamiento:** lanzamiento justo desde el primer día. Software disponible para todo el mundo por adelantado. Todos empiezan en igualdad de condiciones.
- **Emisión:** suave y gradual — la recompensa de minería se reduce a la mitad cada tres años, sin caídas bruscas. Cerca del 21% se mina en el primer año, y alrededor del 90% para el décimo.
- **Unidad base:** 1 MAGLD = 100.000.000 pétalos.
- **Coste de pagar:** 0,01 MAGLD — una centésima de moneda — por cualquier pago cotidiano, envíes lo que envíes. La comisión es un único billete pequeño que se entrega junto con el pago, así que mover un café y mover un auto cuestan lo mismo. Solo las operaciones inusualmente grandes, que agrupan docenas de billetes a la vez, suben a dos o tres centésimas.
- **Comisiones:** todas las comisiones van a los mineros. No se quema nada, no se desvía nada. Una economía de efectivo — donde cada pago es una transacción en la cadena — produce unos ingresos por comisiones constantes que las cadenas de reserva de valor no pueden igualar.

---

## Seguridad en el lanzamiento: ruedas de apoyo que se acaban quitando

Una cadena de prueba de trabajo recién nacida tiene poca potencia de minería, y la poca potencia de minería invita a los ataques. Marigold se lanza con una salvaguarda temporal y enteramente pública llamada **anclas de finalidad**: cinco garantes identificados públicamente co-firman cada cierto tiempo (hacen falta al menos 3 de 5) un bloque reciente y lo vuelven permanente e irreversible. Nadie puede deshacer un bloque anclado.

Los garantes no pueden censurar transacciones, ni crear monedas, ni mover los fondos de nadie, ni producir bloques. Su único poder es impedir que se deshagan transacciones ya completadas. Si se quedan en silencio, la cadena sigue funcionando con normalidad como una red de prueba de trabajo corriente: algo menos protegida, pero plenamente operativa.

La salvaguarda está diseñada para retirarse. Cuando la potencia de minería de la red crezca lo bastante como para que atacarla salga prohibitivamente caro, las anclas pasarán de obligatorias a consultivas y de ahí a caducadas del todo. Antes de que eso ocurra, la elección de los garantes sucesores está pensada para pasar a la gobernanza de quienes tienen billetes — de modo que incluso lo que quede de las ruedas de apoyo pase de los fundadores a la comunidad antes de que se retiren por completo.

---

## Lo que Marigold es — y lo que no

**Marigold es** efectivo digital. Los billetes son instrumentos al portador. Tener la llave es ser el dueño. Entregar la llave es liquidar el pago. La cadena lo dice todo sobre el valor y calla sobre las personas — exactamente igual que un billete de papel.

**Marigold no es** una moneda de privacidad. Nada está cifrado. Nada está oculto. La distinción no es un eufemismo: las monedas de privacidad ocultan criptográficamente los datos que registran, y Marigold no registra ningún dato que ocultar. Lo que comparte con el efectivo es concreto y está honestamente delimitado: el sistema lo dice todo sobre el *valor* por diseño, y calla sobre las *personas* por diseño.

**Marigold es** cinco operaciones, una regla de conservación, un estado a plena vista, firmas digitales corrientes, un suministro fijo que todo el mundo comprueba en todo momento, sobre una red lo bastante rápida como para que entregar un billete Marigold se sienta como entregar un billete de papel.

Todo lo que hace puede comprobarlo la gente para la que está hecho.

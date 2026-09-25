# Litepaper Marigold

## O problema: o dinheiro digital esqueceu o que o dinheiro vivo acertou

Tire uma cédula de dólar da carteira. Repare em três coisas:

1. **Quem está com ela é o dono.** Não existe conta, não existe login, não existe intermediário. A posse é a propriedade.
2. **Ela não tem memória.** A cédula não lembra quem a gastou por último. Não existe registro nenhum ligando você ao café que você comprou hoje de manhã.
3. **Qualquer um consegue conferir se é verdadeira.** Levante contra a luz, sinta o papel. Não precisa de perícia nenhuma.

Nenhuma forma de dinheiro digital em uso hoje reúne as três coisas ao mesmo tempo.

Bitcoin entrega a primeira e a terceira: ter as chaves é ser dono das moedas, e qualquer pessoa pode verificar que o sistema é honesto. Mas falha feio na segunda. Cada Bitcoin já minerado carrega o histórico inteiro, para sempre e à vista de todos. Cada endereço de onde você já mandou ou onde já recebeu fica costurado num grafo que qualquer um pode analisar. Suas moedas lembram de tudo o que você já fez com elas.

Moedas de privacidade como Monero e Zcash tentam resolver isso criptografando o histórico. Elas conseguem esconder — mas a própria criptografia que esconde cria dois problemas novos. Primeiro, quase ninguém consegue verificar por conta própria que o sistema é honesto. Você está confiando em especialistas que nunca viu para garantir que a matemática não tem falha nem porta dos fundos. Segundo, os reguladores dão uma olhada na criptografia complicada e classificam a moeda de acordo — remoção das corretoras, restrições e tratamento hostil vêm não do que a moeda faz, mas de como ela faz.

O resultado é um cenário em que você escolhe entre transparência sem privacidade ou privacidade sem confiança. O dinheiro vivo oferecia as duas coisas, sem esforço nenhum. O dinheiro digital, de algum jeito, esqueceu como se faz.

---

## A ideia: não esconda o rastro — não deixe rastro

Imagine que você está numa barraca de feira. Você entrega uma cédula de cinco dólares ao vendedor. O vendedor põe a cédula no caixa. Mais tarde, no mesmo dia, ele gasta essa mesma cédula na padaria. A padaria dá a cédula de troco para o próximo cliente.

Agora pergunte: quem consegue rastrear essa cédula até você? Ninguém. Não porque o trajeto da cédula tenha sido criptografado ou escondido — a cédula esteve à vista em cada etapa do caminho —, mas porque a própria cédula nunca registrou quem a entregou a quem. A ligação entre você e o vendedor nunca foi anotada. Existiu só no instante da entrega e depois acabou.

Essa é a diferença entre esconder informação e nunca coletá-la. As moedas de privacidade escondem. Marigold não coleta.

---

## Como funciona: cédulas em vez de contas

A maioria das criptomoedas funciona como conta bancária. Você tem um endereço (parecido com um número de conta), e as transações movem valor entre endereços. Os endereços são públicos, e cada movimento entre eles fica registrado para sempre. É o sonho de qualquer contador e o pesadelo da privacidade.

Marigold funciona como dinheiro vivo. Não existem contas. Em vez disso, o sistema mantém um conjunto de **cédulas** — pense nelas como o dinheiro de papel da sua carteira, só que digital.

Cada cédula é um registro público simples:

- Um **número de série** (como o número de série de uma cédula de dólar)
- Um **valor** (0,01, 0,1, 1, 10, 100, 1.000, 10.000 ou 100.000 MAGLD)
- Uma **fechadura** (uma chave pública — quem tiver a chave correspondente pode gastá-la)

É só isso. Nenhum nome de dono, nenhum endereço, nenhuma identidade. A cédula não sabe quem está com ela. Sabe apenas que existe uma chave e que quem conseguir abrir a fechadura pode gastá-la.

### Fazer um pagamento

Pagar alguém é como entregar uma cédula, com um passo a mais:

1. **Você entrega a chave da cédula a quem vai receber** (por QR code, mensagem de texto ou até de viva-voz — do mesmo jeito que você compartilharia uma foto).
2. **Quem recebe troca a fechadura na hora** — registra uma transação que substitui a sua chave por uma chave novinha que só essa pessoa conhece. Isso é confirmado em menos de um segundo.

Confirmada a troca, a cédula é irrevogavelmente de quem recebeu. A sua chave não abre mais nada. A liquidação está feita.

Se quem recebe conseguir mandar a chave nova para você antes (numa cobrança, por exemplo), dá para fazer diferente: você troca a fechadura direto para a chave dessa pessoa. A sua chave não viaja para lugar nenhum.

O terceiro jeito é para pagar alguém que não está ali. Você troca a fechadura por uma chave que só essa pessoa tem e coloca um prazo. Até o prazo, só ela pode pegar a cédula; a partir do prazo, só você pode pegá-la de volta. Ela recebe quando quiser; se nunca receber, o dinheiro volta a ser seu sozinho. Quem faz valer o prazo é a rede, não a carteira de ninguém.

Os três caminhos terminam do mesmo jeito: a cédula fica sob uma chave que só quem recebe conhece.

### Dividir e juntar

As cédulas vêm em valores fixos, igual ao dinheiro de papel. Se você precisa pagar 30 MAGLD e tem uma cédula de 100 MAGLD, você a **divide** em dez cédulas de 10 MAGLD. Entrega três. Fica com sete. Troco — igual ao dinheiro vivo.

No sentido contrário, dez cédulas de 10 MAGLD podem ser **juntadas** em uma cédula de 100 MAGLD. Os valores andam de dez em dez, então qualquer quantia se paga com um punhado de cédulas.

### O que o sistema registra

Toda operação — criar, trocar fechadura, dividir, juntar, resgatar — é pública e está à vista. O que ninguém consegue ver é **quem** fez, porque o sistema não tem a noção de "quem". Não existe campo de remetente. Nem campo de destinatário. Nem agenda de endereços. Nem conta.

O sistema registra que uma cédula mudou de mãos. Não registra de quem eram as mãos. Uma cédula com prazo mostra também as suas condições — o prazo e a chave para a qual ela volta — enquanto o prazo durar.

---

## Marigold no mundo real

Como uma cédula Marigold não passa de uma chave, usá-la no mundo real é tão simples quanto usar dinheiro vivo. Veja como:

### Imprima e gaste

Toda cédula do seu aplicativo de carteira pode ser exibida como um **QR code**. Você pode imprimir esse QR code num pedaço de papel, dobrar e guardar na carteira de couro — bem ao lado dos cartões de crédito e da carteira de motorista. Esse pedaço de papel *é* o dinheiro. Mostre a alguém: a pessoa escaneia, troca a fechadura, e a cédula é dela. A sua chave não funciona mais. O papel que continua no seu bolso agora não vale nada — o valor passou para quem escaneou.

Isso não é metáfora. O QR code impresso guarda a chave de verdade. Perdê-lo é como perder uma cédula de cem dólares: quem achar pode gastar. Guardá-lo bem é a mesma responsabilidade de guardar dinheiro vivo. Não existe central de recuperação, não existe botão de "informar roubo", não existe banco para estornar a transação. É exatamente isso que quer dizer ser ao portador, e é justamente esse o ponto.

### Dê de presente

Um cartão de aniversário com um QR code impresso dentro não se distingue de um cartão de aniversário com uma cédula de cinquenta dólares dentro. Quem recebe escaneia, troca a fechadura na hora, e a cédula é irrevogavelmente dessa pessoa. Nenhuma conta para abrir, nenhum prazo de espera. Abriu o envelope, está com o dinheiro.

### Deixe de herança

Um envelope lacrado no cofre do banco, com os QR codes impressos de várias cédulas, funciona exatamente como um envelope de dinheiro vivo. Quem abre está com as chaves. Quem está com as chaves está com as cédulas. Não é preciso inventariante, nem inventário, nem autorização de terceiros para o valor passar adiante — embora, como acontece com o dinheiro vivo, os arranjos *jurídicos* da herança sejam um assunto à parte, que o sistema não resolve e não tem como resolver. O sistema garante apenas que estar com a chave é estar com a cédula.

### Pague na feira

Você está comprando tomate. O vendedor mostra um QR code — a cobrança dele. Você escaneia com o aplicativo de carteira, escolhe a cédula com que quer pagar, e o seu celular troca a fechadura direto para a chave do vendedor. Menos de um segundo depois, a cédula é dele. Você guarda os tomates. O vendedor nunca viu o seu nome, o seu endereço nem a sua conta. Você nunca viu os dele. A transação está liquidada, é final e pode ser esquecida — igual ao dinheiro vivo.

### Receba enquanto estiver fora

Publique uma única chave — no cartão de visita, na vitrine, no seu perfil — e qualquer pessoa pode pagar você a qualquer hora. Cada pagamento cai sob uma chave nova que só a sua carteira sabe derivar da que você publicou, então a cadeia nunca mostra dois pagamentos chegando ao mesmo lugar. Um pagamento para essa chave pode ter prazo, e assim quem manda sabe que o dinheiro volta se você nunca receber. A sua carteira recebe o que estiver esperando na próxima vez que estiver ligada.

### Guarde a frio

Preocupado com hackers? Imprima as suas cédulas como QR codes, guarde numa caixa à prova de fogo e apague o aplicativo. As cédulas existem no blockchain. As chaves existem no papel. Nenhum aparelho conectado à internet está com elas. Quando quiser gastar, escaneie o QR code de volta para um aplicativo de carteira, troque a fechadura na hora (vai que alguém copiou o papel enquanto ele estava guardado) e transacione normalmente.

### Troque de carteira

Como as cédulas são chaves independentes e não estão amarradas a nenhuma frase-semente ou conta, você pode levar uma cédula de um aplicativo de carteira para outro quando quiser. Exporte a chave de um app, importe no outro. Nenhuma transação no blockchain, nenhuma taxa paga, nenhuma interação com a rede. A sua cédula funciona igual em toda carteira que aceita Marigold — escolha o app de que você gosta, mude quando quiser, o seu dinheiro vai junto.

O fio condutor: uma cédula Marigold é uma chave, e uma chave pode ser impressa, mandada por mensagem, dobrada dentro de um envelope, presa na porta da geladeira ou decorada de cabeça. O sistema não se importa com o caminho que a chave faz entre as pessoas, porque o sistema não sabe que pessoas existem. Ele conhece só chaves e cédulas — e é por isso que toda forma de passar uma cédula de papel adiante tem aqui um equivalente digital direto.

E nada disso precisa de conexão. Bitcoin pode ficar guardado offline como um código QR, mas não pode ser gasto assim: pagar com ele é transmitir uma transação, então alguém precisa estar online, e uma carteira que nunca está online não consegue pagar. Uma cédula Marigold é a própria chave, então pode passar de mão em mão offline, exatamente como uma cédula de papel; a rede só é tocada quando o novo dono resolve trocar a fechadura. Você não precisa estar conectado para pagar, nem de uma carteira permanentemente online.

---

## Seu celular é um controle remoto, não uma carteira

Tudo o que você faz no seu próprio teclado, você faz pelo celular, numa conversa do Telegram: ver o saldo, pagar alguém, receber uma cédula, emitir uma cobrança, consultar o histórico, ver se o seu minerador está rodando. O que muda não é o que você pode fazer. É onde o dinheiro está — e o dinheiro não está no celular.

A sua carteira roda em casa, no que quer que fique ligado: um notebook numa gaveta, um computadorzinho ao lado do roteador. Ela se mantém em dia com o blockchain, guarda as suas cédulas e responde às suas mensagens. O celular no seu bolso não guarda chave nenhuma nem cédula nenhuma. Ele só conversa com o Telegram. Se você perder o celular, perdeu um controle remoto.

O bot é seu, não nosso. Você o cria no Telegram em uns dois minutos, entrega o token à sua carteira e pareia os dois com um código. Daí em diante a sua carteira responde àquela única conta do Telegram e ignora todas as outras. Não existe servidor Marigold no meio, não existe conta conosco, não existe um monte de máquinas nossas de que o seu celular dependa. A sua carteira liga para o Telegram; ninguém liga para ela, e não há nada para abrir no seu roteador.

Gastar é protegido do mesmo jeito que um cartão de banco: uma senha antes de qualquer coisa que mexa com dinheiro, um bloqueio depois de três tentativas erradas que só a máquina de casa consegue liberar, e um limite diário que você mesmo define. Aquela conta do Telegram agora move dinheiro, então precisa de verificação em duas etapas — o bot avisa isso na primeira vez que você fala com ele.

O mesmo chat guarda o seu backup. Assim que você pede uma vez, a carteira publica ali uma cópia criptografada de si mesma e a mantém atualizada sozinha — uma cópia completa por semana, as mudanças poucos minutos depois de um pagamento — em silêncio, sem notificação. Se a máquina de casa morrer, você encaminha essas mensagens para uma carteira nova e está tudo lá. A cópia está trancada com as suas 24 palavras e com mais nada: não com a senha da carteira, que você digita todo dia e escolheu para conseguir lembrar, e que qualquer um com uma cópia do arquivo poderia tentar com toda a calma. As 24 palavras não se adivinham. Anote-as quando a carteira mostrá-las — depois ela pede duas delas para ter certeza — e um backup no servidor de outra pessoa não passa de ruído para todo mundo, menos para você.

Os limites, sem rodeios. Com a máquina de casa desligada, o celular não faz absolutamente nada: nem saldo, nem pagamento, nem recebimento. Um código de pagamento mandado por uma conversa é valor ao portador enquanto está em trânsito, exatamente como os QR codes impressos acima — quem ler primeiro fica com ele. E um aplicativo de celular de verdade, que guarde as chaves ele mesmo, é tarefa para depois e provavelmente para outra pessoa. Esta é a versão que não pede que você confie em ninguém.

Há um segundo jeito de usar o celular, sem bot nenhum: como um lugar para guardar cédulas em forma de imagem. Uma cédula é uma chave, e o código QR dessa chave é o próprio dinheiro; um celular com algumas dessas imagens é uma carteira com algumas cédulas dentro. Trate-as exatamente como dinheiro vivo: quem vê, copia ou escaneia uma delas pode ficar com a cédula colocando a própria fechadura nela, então uma captura de tela num álbum compartilhado é uma cédula esquecida sobre a mesa. O Marigold não tem um aplicativo de celular independente, e é de propósito. A carteira completa roda num computador que você controla, e o celular é o controle remoto dela ou um bolso para cédulas.

---

## O que um observador vê e o que não vê

Marigold não é uma moeda de privacidade, e este litepaper não vai fingir que é. Veja exatamente o que alguém observando o sistema consegue descobrir:

**Dá para ver:** cada cédula, cada valor, cada operação, a oferta total e o estado atual completo do sistema. Tudo isso está à vista. Nada é criptografado, nunca.

**Dá para ver:** a cadeia de operações — qual cédula virou qual cédula nova, quando houve divisões e junções, padrões de horário. Um analista determinado, estudando os registros públicos, consegue identificar padrões de comportamento: pagamentos rápidos em sequência, sequências de dividir e gastar, ou o ritmo diário de um comerciante que recebe e junta cédulas.

**Dá para ver:** quem criou e quem resgatou cédulas, caso consiga ligar um endereço comum de blockchain a uma identidade do mundo real. Os pontos em que Marigold se conecta ao mundo tradicional do blockchain são totalmente visíveis.

**Não dá para ver:** nenhuma identidade ligada a um pagamento dentro do sistema. Não há o que ver, porque o dado nunca foi registrado.

Uma cédula se confunde com todas as outras cédulas do mesmo valor — uma cédula de 100 MAGLD é igual a qualquer outra cédula de 100 MAGLD, e nada além desse grupo. Os aplicativos de carteira podem embaralhar padrões de comportamento (variando um pouco os horários, agrupando operações, alternando os valores de cédula usados para pagar taxas), mas isso é bom hábito, não mágica. Marigold não promete nada além disso.

---

## Por que é dinheiro honesto

A cada poucos segundos, todos os participantes verificam uma única regra:

**Todas as cédulas existentes + todas as moedas do lado tradicional = total de moedas já mineradas.**

Se isso não bater, alguma coisa está errada e a rede enxerga na hora. A oferta presta contas a todo momento — não por confiar num auditor, não por confiar num sistema de provas complicado, mas pela aritmética que qualquer um sabe fazer.

Nenhum valor é criado ou destruído dentro do sistema. Pagar uma taxa tira uma cédula do conjunto, mas o valor dela é creditado ao minerador e pode ser recriado como cédula nova a qualquer momento. A regra de conservação é absoluta e vale para tudo.

---

## O blockchain: rápido o bastante para parecer dinheiro vivo

A regra de liquidação é simples: a cédula é sua quando a sua troca de fechadura é confirmada na rede. Ou seja, **tempo de confirmação é tempo de entrega.** No Bitcoin, você ficaria parado na barraca da feira por dez minutos. No Ethereum, uns doze segundos. No Marigold, menos de um segundo.

Marigold roda em um blockchain de prova de trabalho que produz cerca de 10 blocos por segundo, o que dá confirmação em menos de um segundo. A tecnologia de base foi construída pelo projeto Kaspa — um blockchain rápido, confiável e bem testado. Marigold acrescenta o sistema de cédulas e a economia dele por cima dessa fundação, deixando a camada de base intacta porque ela faz o trabalho dela excepcionalmente bem.

**Uma palavra sobre energia.** A história energética do Marigold não é que ele gasta pouco — um blockchain de prova de trabalho atrai tanta mineração quanto valem as suas recompensas, e um Marigold bem-sucedido não será exceção. A história é que nada dessa energia é gasto para você esperar. Um blockchain clássico só aceita um bloco por rodada; os blocos minerados em paralelo são jogados fora, então a rede só se mantém segura mantendo-se lenta. O blockchain em que Marigold roda guarda todos os blocos — os encontrados no mesmo instante são tecidos juntos no registro, todos eles contando —, e é assim que o mesmo orçamento de segurança entrega dez blocos por segundo e liquidação em menos de um segundo, em vez de uma fila de dez minutos. Por pagamento, isso é uma diferença enorme de eficiência. No total, é a mesma aritmética honesta de todo o resto aqui: energia gasta na proporção do valor que está sendo protegido.

---

## A economia em resumo

- **Oferta:** 210.000.000 MAGLD, teto fixo. Sem pré-mineração, sem fundo de desenvolvimento, sem alocação de espécie alguma.
- **Lançamento:** lançamento justo desde o primeiro dia. Software disponível para todo mundo com antecedência. Todos começam em pé de igualdade.
- **Emissão:** suave e gradual — a recompensa de mineração cai pela metade a cada três anos, sem quedas bruscas. Cerca de 21% é minerado no primeiro ano, e uns 90% até o décimo.
- **Unidade base:** 1 MAGLD = 100.000.000 de pétalas.
- **Custo para pagar:** 0,01 MAGLD — um centésimo de moeda — para qualquer pagamento do dia a dia, por maior que seja a quantia que você está mandando. A taxa é uma única cédula pequena entregue junto com o pagamento, então mover um café e mover um carro custa a mesma coisa. Só operações fora do comum, que empacotam dezenas de cédulas de uma vez, sobem para dois ou três centésimos.
- **Taxas:** todas as taxas vão para os mineradores. Nada é queimado, nada é desviado. Uma economia de dinheiro vivo — em que cada pagamento é uma transação no blockchain — gera uma receita constante de taxas que as redes de reserva de valor não conseguem igualar.

---

## Segurança no lançamento: rodinhas que saem depois

Um blockchain de prova de trabalho novo tem pouco poder de mineração, e pouco poder de mineração é convite para ataque. Marigold é lançado com uma proteção temporária e totalmente divulgada, chamada **âncoras de finalidade**: cinco curadores publicamente identificados assinam em conjunto, de tempos em tempos (com no mínimo 3 de 5), um bloco recente, tornando-o permanente e irreversível. Ninguém consegue desfazer um bloco ancorado.

Os curadores não podem censurar transações, criar moedas, mover o dinheiro de ninguém nem produzir blocos. O único poder deles é impedir que transações concluídas sejam desfeitas. Se ficarem em silêncio, o blockchain segue normalmente como uma rede comum de prova de trabalho — um pouco menos protegida, mas plenamente operacional.

A proteção foi feita para se aposentar. Quando o poder de mineração da rede crescer a ponto de tornar os ataques proibitivamente caros, as âncoras deixam de ser obrigatórias, passam a ser apenas consultivas e depois expiram de vez. Antes disso, a escolha dos curadores sucessores deve passar para a governança de quem tem cédulas — de modo que até o que resta das rodinhas sai das mãos dos fundadores para as da comunidade antes de ser retirado por completo.

Na rede de testes pública a proteção já está funcionando: cinco chaves de curadores, uma âncora a cada trinta segundos mais ou menos, todos os nós fazendo valer. Os curadores da rede de testes são chaves descartáveis feitas para o ensaio; os de verdade são escolhidos no lançamento.

---

## O que Marigold é — e o que não é

**Marigold é** dinheiro vivo digital. As cédulas são títulos ao portador. Estar com a chave é ser dono. Passar a chave adiante é liquidar. O blockchain é completo sobre o valor e silencioso sobre as pessoas — exatamente como uma cédula de dólar.

**Marigold não é** uma moeda de privacidade. Nada é criptografado. Nada é escondido. A distinção não é eufemismo: as moedas de privacidade escondem, com criptografia, dados que registraram; Marigold não registra dado nenhum para esconder. O que ele tem em comum com o dinheiro vivo é específico e honestamente delimitado — o sistema é, deliberadamente, completo sobre o *valor* e, deliberadamente, silencioso sobre as *pessoas*.

**Marigold é** cinco operações, uma regra de conservação, estado à vista de todos, assinaturas digitais comuns, uma oferta fixa conferida por todo mundo a todo momento, numa rede rápida o bastante para que entregar uma cédula digital pareça entregar uma cédula de papel.

Tudo o que ele faz pode ser conferido pelas pessoas para quem ele existe.

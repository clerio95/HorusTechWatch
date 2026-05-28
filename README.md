# HorusTechWatch

Publicador de estado **somente leitura** para o concentrador de automação de bombas de combustível **Companytec Horustech**, utilizado em postos de gasolina.

## O que faz

O HorusTechWatch consulta um concentrador Horustech ativo via TCP e publica atomicamente um arquivo `state.json` em um compartilhamento de rede Windows. Uma ferramenta de alarme de terceiros lê esse arquivo para decidir o que alertar. O sistema é deliberadamente **somente leitura** — nunca escreve no dispositivo, nunca incrementa ponteiros de abastecimento e nunca envia quadros de controle.

Cada ciclo de consulta executa cinco comandos permitidos contra o dispositivo:

| Comando | O que retorna |
|---------|---------------|
| **Status** | Estado por bico (`B`=bloqueado, `L`=livre, `A`=abastecendo, `F`=falha, …) |
| **Info do dispositivo** | Versão de firmware, nível e tensão da bateria, MAC, IP, número de série |
| **Relógio** | Data/hora do dispositivo (para detecção de desvio) |
| **Diagnóstico** | Status por bomba (`R`=respondendo, `F`=falha, `N`=não configurado) |
| **Diag. sem fio** | Qualidade de sinal e RSSI por bomba |

## Arquitetura

```
main.rs         loop de consulta, despacho
config.rs       carregador TOML — aplica piso de 30 s no intervalo
audit.rs        log de auditoria JSONL diário (logs/YYYY-MM-DD.jsonl)
state.rs        StateAccumulator + escrita atômica (tmp → rename)
client.rs       TCP síncrono, timeout de 5 s, leitor de quadros limitado
protocol/       checksum, montador/parser de quadros — núcleo crítico de segurança
parse/          um módulo por tipo de resposta de comando
```

Consultas com falha preservam o último valor válido conhecido, para que os consumidores possam verificar a defasagem de cada campo. O enum `Command` torna os índices proibidos do dispositivo irrepresentáveis no nível de tipos.

## Restrições de segurança (fixas, não configuráveis)

- **Piso do intervalo de consulta:** 30 segundos — valores abaixo disso são rejeitados na inicialização
- **Intervalo padrão:** 60 segundos
- **Timeout de socket:** 5 segundos por leitura
- **Lista de comandos permitidos:** apenas os índices `{0x01, 0x0B, 0x12, 0x1B, 0x25}` podem ser enviados — qualquer outro causa pânico

## Compilação

```sh
# Testes + binário nativo
cargo test && cargo build --release

# Cross-compile para Windows
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
# → target/x86_64-pc-windows-gnu/release/horustechwatch.exe (~570 KB)
```

## Configuração

Copie e edite o `config.toml`. Campos principais:

```toml
[device]
ip   = "192.168.25.91"
port = 2001

[output]
# Use strings com aspas simples para caminhos Windows (barra invertida é literal em TOML com aspas simples)
state_file = '\\Servidor\pista\Relátório-pista\modulo_alarmes\concentrador\state.json'

[poll]
interval_seconds = 60   # mínimo 30; valores abaixo de 30 são rejeitados na inicialização
```

## Implantação

Em produção desde **27/05/2026** no host Windows `PST-ZAM-04`, publicando em `\\Servidor\pista\Relátório-pista\modulo_alarmes\concentrador\state.json`.

## Créditos

Protocolo de comunicação implementado a partir do manual **DT214 Rev.14**, disponibilizado pela Companytec através do seu kit de desenvolvimento aberto:

> [companytec/companytec_kit_desenvolvimento](https://github.com/companytec/companytec_kit_desenvolvimento)

Esse repositório forneceu a especificação do protocolo, as regras de enquadramento, o algoritmo de checksum e a referência de comandos que são a base deste projeto.

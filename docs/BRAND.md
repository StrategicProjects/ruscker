# Ruscker — Diretrizes de marca

Sistema visual baseado na pilha isométrica de containers, evocando
réplicas balanceadas. Este documento descreve quando usar cada
arquivo, regras de aplicação e proporções de lockup.

## Arquivos

| Arquivo | Uso |
|---|---|
| `ruscker-mark.svg` | Marca primária colorida — uso principal em backgrounds claros |
| `ruscker-mark-flat.svg` | Variante plana (3 barras) para favicons em 16-24 px |
| `ruscker-mark-mono-black.svg` | Versão preta para impressão monocromática |
| `ruscker-mark-knockout.svg` | Versão branca para uso sobre fundos escuros |
| `ruscker-app-icon.svg` | Ícone de app (quadrado arredondado, fundo teal 600, marca branca) |
| `ruscker-lockup-horizontal.svg` | Marca + wordmark lado a lado — uso em headers, papelaria |
| `ruscker-lockup-horizontal-dark.svg` | Igual, com o wordmark branco — uso sobre fundos escuros (README dark, tema escuro da doc) |
| `ruscker-lockup-vertical.svg` | Marca acima do wordmark — uso em avatares, social, app store |
| `ruscker-wordmark.svg` | Só o texto — uso quando o ícone seria redundante (em watermarks, footer minimalista) |

## Paleta

| Token | Hex | Uso |
|---|---|---|
| teal 200 | `#5DCAA5` | Camada superior da pilha, hover states, acentos suaves |
| teal 400 | `#1D9E75` | Camada do meio, links primários, badges |
| **teal 600** | **`#0F6E56`** | **Primária da marca**: base da pilha, fundo do app icon, CTAs |
| teal 800 | `#085041` | Texto sobre fundos claros teal, hover de CTAs |

A marca primária é **teal 600**. As outras camadas existem para o
gradiente isométrico da marca e para variações de UI.

## Tipografia

- **Wordmark**: Jost Medium (peso 500), letter-spacing `-0.02em`,
  caixa baixa.
- Sem alternativa: o wordmark sempre em caixa baixa. Nunca "RUSCKER"
  nem "Ruscker".

## Lockup — proporções

**Horizontal**: ícone à esquerda, wordmark à direita.
- altura do ícone = 1×
- altura visual do wordmark (cap-height) ≈ 0.55× ícone
- gap entre ícone e texto = 0.15× altura do ícone
- alinhamento vertical: óptico (centros visuais alinhados, não os
  bounding boxes)

**Vertical**: ícone acima, wordmark abaixo.
- altura do ícone = 1×
- altura visual do wordmark ≈ 0.40× ícone
- gap = 0.15× altura do ícone

## Espaço de respiro

Reserve sempre, ao redor da marca, uma margem mínima igual à **altura
de uma camada da pilha** (1/3 da altura total do ícone). Nada de outros
elementos visuais — texto, logos, bordas — pode invadir essa zona.

## Tamanhos mínimos

- Marca isométrica: 24 px de altura.
- Marca plana (3 barras): 16 px de altura.
- Lockup horizontal: 28 px de altura (ícone + wordmark).
- App icon: 40 px (no mínimo, para o `rx="18"` ainda parecer
  arredondado).

Abaixo dos mínimos, escolher a variante plana ou apenas o wordmark.

## O que NÃO fazer

- Não rotacionar, distorcer ou inclinar a marca.
- Não trocar as cores fora da paleta. Verde-teal é a identidade — não
  use roxo, azul ou laranja.
- Não inverter a ordem das camadas (claro embaixo, escuro em cima).
- Não aplicar sombras, gradientes, ou efeitos de relevo.
- Não usar sobre backgrounds de baixo contraste ou padronados sem
  espaço de respiro adicional.
- Não combinar com outros logos dentro da zona de respiro.
- Não recortar partes da marca para "compor" outra coisa.

## Notas de produção

Os arquivos de lockup (`ruscker-lockup-horizontal.svg` e
`ruscker-lockup-vertical.svg`) e o wordmark
(`ruscker-wordmark.svg`) usam `<text>` apontando para Jost via
fallback do sistema. Para produção (impressão, e-mail HTML, sites de
terceiros):

1. Abrir o SVG no Figma ou Illustrator.
2. Converter o `<text>` em paths/outlines.
3. Re-exportar como SVG.

Isso garante consistência cross-plataforma, independente da fonte
estar instalada ou não.

## Para investidores e parceiros

Use o **lockup horizontal** em apresentações, papelaria e assinatura
de e-mail. Use o **app icon** em redes sociais (LinkedIn, Twitter).
Use o **wordmark** quando a marca já apareceu na mesma página e o
ícone seria repetição.

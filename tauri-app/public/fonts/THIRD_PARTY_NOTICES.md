# Bundled font notices

The desktop application bundles the following font files. Each font is
redistributed under the SIL Open Font License, Version 1.1, independently of the
application's GPL-3.0 license. The accompanying upstream license text for each
font family is stored in [`licenses/`](licenses/).

**Packaging note.** The two stacks share one font source in the repository root
(`public/fonts/`). The Tauri build ships these fonts re-packaged as WOFF2
(Brotli) instead of the upstream TrueType files; glyph outlines, metrics and
font names are unchanged, and the conversion is a lossless re-containerization
of the same releases listed below. Hashes refer to the files actually shipped.

## Inter 4.001

Copyright 2020 The Inter Project Authors (https://github.com/rsms/inter).

Official source: [Google Fonts download manifest](https://fonts.google.com/download/list?family=Inter)

- `Inter-VariableFont_opsz,wght.woff2` — SHA-256 `bff0f2e92cf937b8f279fdcc2bc40cdcf6668ce0cc5fd74de5f3738f674a4676`
- `Inter-Italic-VariableFont_opsz,wght.woff2` — SHA-256 `e020c1861590ade77a9795eaba21aba572d0c4f2161ed4d3b4d355e1a3fb426a`
- License: [`licenses/Inter-OFL-1.1.txt`](licenses/Inter-OFL-1.1.txt)

## LXGW WenKai 1.522

Copyright 2021-2026 LXGW (https://github.com/lxgw/LxgwWenKai), with the
Reserved Font Names and additional permission stated in the accompanying
license. Copyright 2020 The Klee Project Authors
(https://github.com/fontworks-fonts/Klee).

Official source: [LXGW WenKai commit `50f4b182415a8c33d9a456df220b66a284e2509b`](https://github.com/lxgw/LxgwWenKai/tree/50f4b182415a8c33d9a456df220b66a284e2509b)

- `LXGWWenKai-Regular.woff2` — SHA-256 `8c92d1b42fbfc8e6f81eb57810d58c85106e8682d1d05e9cda31552326e440d3`
- `LXGWWenKai-Medium.woff2` — SHA-256 `ec2022329b12de91682436d5ddb0353b71d5cfffcaa83763b3861ad898e1047f`
- License and additional permission: [`licenses/LXGWWenKai-OFL-1.1.txt`](licenses/LXGWWenKai-OFL-1.1.txt)

## Noto Sans SC 2.004-H2

Copyright 2014-2021 Adobe (http://www.adobe.com/), with Reserved Font Name
"Source".

Official source: [Google Fonts download manifest](https://fonts.google.com/download/list?family=Noto%20Sans%20SC)

- `NotoSansSC-VariableFont_wght.woff2` — SHA-256 `8b3d500e763c7a893be2d6bd0d64a7b39e0e9fdf366e2141ef4d9b9fdbc5e50c`
- License: [`licenses/NotoSansSC-OFL-1.1.txt`](licenses/NotoSansSC-OFL-1.1.txt)

## Noto Serif SC 2.003-H1

Copyright 2012 Google Inc. All Rights Reserved.

Official source: [Google Fonts download manifest](https://fonts.google.com/download/list?family=Noto%20Serif%20SC)

- `NotoSerifSC-VariableFont_wght.woff2` — SHA-256 `58e7c38bdd33d908ab579edba323de99e9dc2cb85b44f405a82999d40c715603`
- License: [`licenses/NotoSerifSC-OFL-1.1.txt`](licenses/NotoSerifSC-OFL-1.1.txt)

## Instrument Serif（Google Fonts v5）

Copyright 2022 The Instrument Serif Project Authors (https://github.com/Instrument/instrument-serif)。

随上游纸墨外壳分发的 Fontsource `@fontsource/instrument-serif` 5.3.0 拉丁字形 WOFF2，用于章节及阶段数字。文件仅重命名，未做额外裁剪或转换，不替代写作所需的完整中文字体。

- `InstrumentSerif-Latin-Regular.woff2` — SHA-256 `5eb09b5ac0e28b67c2f041c8ba6d244604ca0c0980d65912ab2d47fed84ddc31`
- `InstrumentSerif-Latin-Italic.woff2` — SHA-256 `5a51946dfffa82972bc98745359c46761515641fda557c25116459a9f83da4a7`
- 许可证：[`licenses/InstrumentSerif-OFL-1.1.txt`](licenses/InstrumentSerif-OFL-1.1.txt)

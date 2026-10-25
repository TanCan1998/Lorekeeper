# LoreKeeper 定稿图标

用户已采用单颗彩色四芒星与手绘“司”字组合，默认亮色，保留亮暗两套。此处只登记选中资源，不包含旧候选、用户参考图或第三方官方图。

## 正式资源

- `tauri-app/public/branding/app-icon-light.svg`：默认亮色源图。
- `tauri-app/public/branding/app-icon-dark.svg`：暗色配套源图。
- `tauri-app/public/branding/app-icon-light-small.svg`：**16–32px 专用简化稿**（星芒改为四块实色并放大到 `scale(3.2)`，「司」放大 1.1 倍），只用于桌面小尺寸帧。
- `tauri-app/src-tauri/icons/`：从亮色源图生成的五份 PNG、Windows ICO、macOS ICNS，沿用现有配置路径。
- 启动页、顶栏、关于页复用上述 SVG；应用内随亮暗主题切换，桌面安装图标默认亮色。

口部星芒以原中心 `(106.5, 163)` 等比放大 2.5 倍，宽高由 28 增至 70；渐变随轮廓同步缩放，上下尖角各超出口部 11 个源图单位。亮暗源图与桌面资源保持一致。

## 小尺寸帧（16–32px）

整图等比缩到 16–32px 时，「司」与四色星芒会糊成文档状（2026-10-25 覆盖安装核验时发现：安装后 32px 图标与旧 PageKin 图标肉眼难分）。因此 `icon.ico` 按尺寸取两套源图：

```powershell
# 1) 两套源图各自生成 ICO
pnpm tauri icon public/branding/app-icon-light.svg --output ../.pi/tmp/icons/full
pnpm tauri icon public/branding/app-icon-light-small.svg --output ../.pi/tmp/icons/small
# 2) 16/24/32 取简化稿，48/64/256 取完整稿
pnpm icons:compose -- --small ../.pi/tmp/icons/small/icon.ico --full ../.pi/tmp/icons/full/icon.ico --out src-tauri/icons/icon.ico
```

`src-tauri/icons/32x32.png` 同步用简化稿的 32px PNG；48px 以上的 PNG 与 ICNS 仍用完整稿。组合逻辑与失败路径由 `scripts/icon-frames.node-test.mjs` 覆盖（`pnpm test:contract`）。

源图 SHA256：

```text
711d257aec83dfc06e7426222cc247e196e02701c053ee4332b6e622bc6bd8c5  app-icon-light.svg
63e84a92f2534bfdc9df7cfc6ea36dea5ca5bb692d8fcaa6aef6a0853d214fa8  app-icon-dark.svg
f353d2f3c6db7a84fd7b1c44660adba955c1012bc3888cabfe27ac81981731e5  app-icon-light-small.svg
821cf027f21fa0019d990f4a7f4016b0b949b1925dc50273ec14618f91b5c97e  icons/icon.ico（组合结果）
```

## 生成与验证

在 `tauri-app/` 使用项目已安装的 Tauri CLI，不需要新增字体或依赖：

```powershell
pnpm tauri icon public/branding/app-icon-light.svg --output ../.pi/tmp/branding-adoption/light
```

只将 `32x32.png`、`64x64.png`、`128x128.png`、`128x128@2x.png`、`icon.png`、`icon.ico`、`icon.icns` 七份桌面资源接入 `src-tauri/icons/`，不提交临时生成目录及移动端资产。源图摘要、单星轮廓、渲染资源选择、PNG尺寸、ICO/ICNS结构由 `app-icon.test.ts` 覆盖。

本切片不改变产品名、identifier、版本、数据目录或安装升级身份。旧产品名 `Lorekeeper` 对应的 WiX UpgradeCode 已通过 CLI 查询确认：`ed3e3199-4ddc-501a-a745-179a185e8dfb`；将来调整大小写前仍须显式保留该值。

图标接入**不是**WebView子进程归组、安装卸载认证或新版本发布完成的证据。包内资源与真实桌面图标缓存行为仍需后续打包及后台验收。

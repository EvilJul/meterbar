# Meterbar icons

| 源 | 用途 |
|---|---|
| `meterbar-app.svg` | App 图标母版（深底 + 钢蓝/绿弧） |
| `meterbar-tray.svg` | 菜单栏 template（黑 + 透明） |

成品准星：`../icon-1024.png`、`../tray-icon.png`。

## 导出（macOS）

```bash
# 需 Inkscape 或 rsvg-convert；也可用设计工具从 SVG 导出 PNG
# App 母版 → 1024
rsvg-convert -w 1024 -h 1024 meterbar-app.svg -o ../icon-1024.png

# 常用尺寸
for s in 32 64 128 256 512; do
  sips -z $s $s ../icon-1024.png --out ../${s}x${s}.png
done
sips -z 256 256 ../icon-1024.png --out ../128x128@2x.png
sips -z 512 512 ../icon-1024.png --out ../icon.png

# icns：准备 icon.iconset 后
# iconutil -c icns icon.iconset -o ../icon.icns

# Tray template（1x/2x）
rsvg-convert -w 22 -h 22 meterbar-tray.svg -o ../tray-iconTemplate.png
rsvg-convert -w 44 -h 44 meterbar-tray.svg -o ../tray-iconTemplate@2x.png
cp ../tray-iconTemplate@2x.png ../tray-icon.png   # lib.rs include_bytes!
```

`tauri.conf.json` 引用：`32x32` / `128x128` / `128x128@2x` / `icon.icns` / `icon.ico`。  
Tray 运行时：`src-tauri/src/lib.rs` → `icons/tray-icon.png`（必须入库）。

# Longhand icons

`src/assets/longhand-mark.svg` defines the L and pen dot once. The in-app
emblem uses it as a CSS mask with a theme-specific gradient and a transparent
rounded outline. The header moves the mark down four SVG units to center it
inside that outline.

The macOS and Windows app icons use the same cream gradient on a navy tile.
The operating system can add its own rendering effects. The tray stays a
monochrome template. Its tighter viewBox makes the mark 25% larger than the
original inside the same 22-point slot.

Regenerate into a temporary directory, then copy only the desktop assets:

```sh
uv run scripts/icons/make_icons.py /tmp/longhand-icons
bun tauri icon /tmp/longhand-icons/app-icon.png --output /tmp/longhand-icons
cp /tmp/longhand-icons/{app-icon.png,tray.png,32x32.png,64x64.png,128x128.png,128x128@2x.png,icon.png,icon.icns,icon.ico} src-tauri/icons/
```

The Python script pins its rasterizer and Pillow versions. Do not copy the
generated Android, iOS, or Windows Store assets; Longhand does not ship them.

The approved UI polish design adds gradients and the outline to the original
plan's plain foreground mark. It applies the gradient to both desktop icons
and increases the tray mark. The original mark geometry stays unchanged.

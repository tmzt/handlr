# The companion's launcher icon

Findings and previews of the companion app's launcher icon (`apps/companion/icon/`).
This document is append-only (AGENTS.md Rule 3.6): each addition is a dated section,
and a correction is a later section. Images are linked relative to the repository
root, as repowiki serves them.

## 2026-10-24 14:02 EDT — the magenta circuit dog

The source is the left icon of `~/Projects/Handlr/Gemini_Generated_Image_aq11hwaq11hwaq11.jpeg`
(not committed): a line-art dog of circuit traces in a blue-to-green gradient, on a
rounded navy square. `make_icon.py` crops it, turns the line art's hue by +110° (hue
155–210° becomes about 265–320°: the tail goes from blue to magenta, the head from green
to violet), redraws the navy square without the JPEG's white fringe, and writes every
size with Lanczos downscaling. The background is `#0F0F26`, the source navy (`#0A1025`)
moved slightly towards purple. Shifts of +90 and +105 left the head blue-violet; +120
and more pushed the tail into hot pink.

The master, 512 px:

![The launcher icon at 512 px](./apps/companion/icon/icon-512.png)

The adaptive icon's foreground, the dog alone on transparency. Launchers mask it into
their own shape over the navy background; the dog sits inside the 66 dp circle every
mask keeps:

![The adaptive foreground at 432 px](./apps/companion/icon/android/res/mipmap-xxxhdpi/ic_launcher_foreground.png)

The legacy and round icons, at each density (48, 72, 96, 144, 192 px):

| Density | Square | Round |
|---|---|---|
| mdpi, 48 px | ![mdpi](./apps/companion/icon/android/res/mipmap-mdpi/ic_launcher.png) | ![mdpi round](./apps/companion/icon/android/res/mipmap-mdpi/ic_launcher_round.png) |
| hdpi, 72 px | ![hdpi](./apps/companion/icon/android/res/mipmap-hdpi/ic_launcher.png) | ![hdpi round](./apps/companion/icon/android/res/mipmap-hdpi/ic_launcher_round.png) |
| xhdpi, 96 px | ![xhdpi](./apps/companion/icon/android/res/mipmap-xhdpi/ic_launcher.png) | ![xhdpi round](./apps/companion/icon/android/res/mipmap-xhdpi/ic_launcher_round.png) |
| xxhdpi, 144 px | ![xxhdpi](./apps/companion/icon/android/res/mipmap-xxhdpi/ic_launcher.png) | ![xxhdpi round](./apps/companion/icon/android/res/mipmap-xxhdpi/ic_launcher_round.png) |
| xxxhdpi, 192 px | ![xxxhdpi](./apps/companion/icon/android/res/mipmap-xxxhdpi/ic_launcher.png) | ![xxxhdpi round](./apps/companion/icon/android/res/mipmap-xxxhdpi/ic_launcher_round.png) |

The contact sheet `make_icon.py` writes, with every size on light and dark backdrops and
the adaptive icon under a circle and a rounded mask:

![The icon contact sheet](./apps/companion/icon/preview.png)

In the APK, `aapt dump badging` gives `icon='res/mipmap-anydpi-v26/ic_launcher.xml'`, and
the manifest's `android:icon` and `android:roundIcon` are `mipmap/ic_launcher` and
`mipmap/ic_launcher_round`. They come in through libmobile's `Project::extra_script`
(`apps/companion/icon/android/icon.gradle`) until libmobile and libmorph's icon plans
give `Project::icon(..)`.

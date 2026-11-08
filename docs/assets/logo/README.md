# Antibody logo files

The mark is an antibody holding its antigen, a diamond, with one constant gap all round. The same diamond is the
dot of the i in the wordmark. Below 48 px the mark uses its small-size cut, whose wider gap keeps the diamond apart
from the arms. The wordmark is Instrument Sans at weight 640, tracked −0.01 em, outlined.

Colours: ink `#0c0e0f` and accent `#d6156f` on light backgrounds; ink `#edeeeb` and accent `#ff3f98` on dark ones.
The app icons' tile is `#0c0e0f`.

## svg/

| File | Use |
|---|---|
| `antibody-mark-light.svg`, `antibody-mark-dark.svg` | The mark (primary cut), for light or dark backgrounds |
| `antibody-mark-small-light.svg`, `antibody-mark-small-dark.svg` | The mark's small-size cut, for anything under 48 px |
| `antibody-mark-mono-black.svg`, `antibody-mark-mono-white.svg` | The mark in one colour, where the accent cannot be used |
| `antibody-lockup-light.svg`, `antibody-lockup-dark.svg` | The mark and the wordmark together |
| `antibody-lockup-small-light.svg`, `antibody-lockup-small-dark.svg` | The lockup with the mark's small-size cut, for small heights (the site's header uses it) |
| `antibody-wordmark-light.svg`, `antibody-wordmark-dark.svg` | The wordmark alone |
| `antibody-app-icon-light.svg`, `antibody-app-icon-dark.svg` | The mark on a rounded tile |
| `geometry.json` | The mark's paths (both cuts), the lockups' view boxes and the colours |

## png/

| File | Use |
|---|---|
| `antibody-mark-light-1024.png`, `antibody-mark-dark-1024.png` | The mark at 1024 px |
| `antibody-lockup-light-1200.png`, `antibody-lockup-dark-1200.png` | The lockup at 1200 px wide |
| `antibody-icon-192.png`, `antibody-icon-512.png` | App icons (dark tile) |
| `antibody-avatar-400.png` | An avatar for profiles |

The site's own copies are in `web/`: `app/icon.svg` (the favicon, light or dark by the system setting),
`app/favicon.ico` (16, 32 and 48 px), `app/apple-icon.png` (180 px), `components/ui/Logo.tsx` (`LogoMark` and
`Wordmark`) and `lib/og-logo.tsx` (`OgLockup`, for the share images).

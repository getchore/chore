# design

Source for the social images. Two jobs, two shapes.

**Link preview** — what X, Telegram, Slack and Discord pull in when someone
pastes the site URL. Small on screen, so it carries the install line and the
brand.

| file | what it is |
| --- | --- |
| `og.html` | the card, laid out at exactly 1200×630 |
| `og.png` / `og@2x.png` | 1200×630 and 2400×1260 |

**In-feed images** — attached to a tweet by hand. The tweet already carries the
link and the pitch, so these drop the URL and go big: one terminal, 26px type,
readable at feed size.

| file | what it is |
| --- | --- |
| `x.html` | `chore build` running through the builtins |
| `x-check.html` | `chore check` catching what breaks on Windows |
| `x.css` | shared styling for both |
| `x.png`, `x-check.png` | 1600×900 (`@2x` at 3200×1800) |

| | |
| --- | --- |
| `fonts.css` | Inter and JetBrains Mono, latin subsets, inlined as base64 so a render needs no network |
| `render.sh` | screenshots every card with headless Chrome |

## Regenerating

```sh
sh design/render.sh
```

It writes every PNG here and copies the 1× one to `website/public/og.png`,
which `website/index.html` points at through `og:image` and `twitter:image`.
Set `CHROME=/path/to/chrome` if Chrome is not in `/Applications`.

The version pill on the link-preview card is hardcoded in `og.html`; bump it
there and re-render when it goes stale.

## Refreshing the caches

X and Telegram cache preview images per URL for a long time. After deploying a
new `og.png`, re-scrape with the
[X card validator](https://cards-dev.twitter.com/validator), and for Telegram
send `/webpage https://getchore.github.io/chore/` to
[@WebpageBot](https://t.me/WebpageBot).

# design

Source for the social preview card — the large image X, Telegram, Slack and
Discord show when someone pastes a link to the site.

| file | what it is |
| --- | --- |
| `og.html` | the card, laid out at exactly 1200×630 |
| `fonts.css` | Inter and JetBrains Mono, latin subsets, inlined as base64 so a render needs no network |
| `og.png` | 1200×630, the file the site links to |
| `og@2x.png` | 2400×1260, for anywhere that wants the retina copy |
| `render.sh` | screenshots `og.html` with headless Chrome |

## Regenerating

```sh
sh design/render.sh
```

It writes both PNGs here and copies the 1× one to `website/public/og.png`,
which `website/index.html` points at through `og:image` and `twitter:image`.
Set `CHROME=/path/to/chrome` if Chrome is not in `/Applications`.

The version pill in the top-left is hardcoded in `og.html`; bump it there and
re-render when it goes stale.

## Refreshing the caches

X and Telegram cache preview images per URL for a long time. After deploying a
new `og.png`, re-scrape with the
[X card validator](https://cards-dev.twitter.com/validator), and for Telegram
send `/webpage https://getchore.github.io/chore/` to
[@WebpageBot](https://t.me/WebpageBot).

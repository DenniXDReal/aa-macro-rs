# aa-macro-rs

Anime Adventures macro in Rust. It's a clean-room rewrite built to run **up to 4 Roblox clients at once**, either as independent farms or as a host plus followers in one party, and it only talks to servers you own.

> Private repo. Roblox treats macros as a ToS issue, so use alt accounts.

## Download

GitHub → **Actions** → latest green **build** run → **Artifacts** → `aa-macro-windows` (zip with `aa-macro.exe`).

## Phase 1 tools (multi-instance plumbing)

Run these from a terminal in the folder with `aa-macro.exe`:

| Command | What it does |
| --- | --- |
| `aa-macro unlock` | Holds Roblox's single-instance lock. **Start it before opening the first Roblox** and keep it open, then open up to 4 clients. |
| `aa-macro list` | Lists the Roblox clients it found (hwnd, pid, client size). |
| `aa-macro tile` | Resizes every client to an 800×599 viewport and tiles them. On 1080p they overlap a bit, which is fine. |
| `aa-macro doctor` | Checks Windows scaling (must be 100%), client size and capture for each client. |
| `aa-macro capture` | Saves `captures/instanceN.png` for every client, even covered ones. |
| `aa-macro ping` | Hovers each client's centre in turn through the input arbiter. It sends no clicks and no keys. Shows focus switching works. |

The first run writes `aa-macro.json` next to the exe (settings).

## Patterns (Phase 2): teach it what buttons look like

Patterns are **yours**, captured from your own game. They live in `patterns/` next to the exe.

1. Get to the screen with the button (e.g. the vote screen) with **no pop-ups covering it**, then run `aa-macro capture`.
2. Open `captures/instance1.png` in **Paint**. Hover the top-left corner of the button text: Paint shows `x, y` at the bottom left. Drag a selection over the text: Paint shows its `w × h`.
3. Save it:
   ```
   aa-macro pattern add "Vote Start" --rect 330,52,140,32
   ```
   * Text and labels: the default black/white pattern. Check `patterns/vote_start.preview.png` and make sure the text is readable. If it isn't, add `--threshold 150` (or another value from 0 to 255).
   * Coloured icons: add `--image`.
   * It searches 40 px around the crop later. Use `--margin 100` for buttons that move.
4. Test on a live client: `aa-macro pattern test` (all patterns) or `aa-macro pattern test "Vote Start" --instance 2`.
   `--from captures/x.png` works on saved screenshots, and `--everywhere` searches the whole client.

Speed: a pattern search in its own box takes well under 1 ms. A worst-case whole-client search takes about 20 ms.

### Phase 1 test checklist

1. Close all Roblox. Run `aa-macro unlock` in one terminal.
2. Open 2–4 Roblox clients (alts) into Anime Adventures and leave them in the lobby, windowed.
3. In a second terminal: `aa-macro list`, then `tile`, then `doctor`, then `capture`, then `ping`.
4. Send the output and the `captures/` PNGs back.

## Design

```
cli / gui
  -> instances: one worker + state machine per client        (phase 3+)
    -> input::Arbiter: the ONLY owner of mouse + keyboard
    -> capture: per-window PrintWindow frames (works when covered)
      -> platform::Platform: Win32 / Null / Mock (tests)
```

Roblox only reads real input, and real input goes to the focused window. Workers never send input themselves. They submit short batches (`press 2 → click (412,300) → press q`). The arbiter runs one batch at a time: highest priority first, focus that window, run it, then release every key it pressed, including on stop or pause. Detection runs in parallel per client, and only input takes turns.

All coordinates are **client space** (800×599 viewport, (0,0) top-left).

## Roadmap

1. ✅ Platform, input arbiter, capture, tiling, unlock, CLI, Windows CI
2. ✅ Pattern matcher (text + colour) and `pattern add/test`. OCR is next (2b)
3. Story / Legend / Raid loop, placement, upgrades
4. Dungeon, Infinity Castle, Challenge
5. Independent + Party (host/follower) multi modes
6. Discord webhook (direct) + own API (Cloudflare Worker on `api.dennixd.org/aa/*`)
7. Summer event mode (needs screenshots)

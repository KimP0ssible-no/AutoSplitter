# AutoSplitter

AutoSplitter is a speedrun timer for Windows that splits by itself. It reads the text a game shows on screen, for example a mission objective. When that text matches a keyword in your profile, AutoSplitter starts the timer or moves to the next split. It was written for Tom Clancy's The Division 2, and it works with any game that shows text you can match.

AutoSplitter runs on its own or sends its splits to [LiveSplit](https://livesplit.org).

The [guide](docs/guide.md) explains every window, setting and profile attribute. The [changelog](CHANGELOG.md) lists what changed in each version, back to 0.1.

## What it does

- Reads a screen region you choose with OCR (optical character recognition, reading text from an image). It uses the PP-OCR models from PaddleOCR on ONNX Runtime, on the CPU, in about 20 to 50 ms per read.
- Matches each segment's keywords with a fuzzy match, so one or two misread letters still match.
- Times each split from the first captured frame that showed the text, not from the moment OCR read it: within one capture, 200 ms at the default 5 frames per second.
- Shows the splits, the timer and your personal best (PB) in its window and in an overlay that stays above the game.
- Sends start, split, skip and reset to LiveSplit Server.
- Saves your PB in the profile when a run ends, and asks first.
- Reads English and Simplified Chinese.
- Comes with a profile editor that shows the live OCR text, so you can copy it into keywords.

## Download

Download the newest version from the [releases page](https://github.com/KimP0ssible-no/AutoSplitter/releases/latest). AutoSplitter needs Windows 10 or 11, 64-bit.

| File | Use |
| --- | --- |
| `autosplitter-X.Y.Z-x64-setup.exe` | The installer. It adds a Start menu entry and an uninstaller. |
| `autosplitter-X.Y.Z-x64-portable.zip` | The portable version. Unpack it into any folder and run `AutoSplitter.exe`. |

The installer keeps your profiles in `%APPDATA%\AutoSplitter\profiles`. The portable version keeps them in the `profiles` folder beside `AutoSplitter.exe`, because of the `portable.txt` file there. The uninstaller and updates never delete or replace profiles.

### Update from version 1

AutoSplitter 1.x was published on autosplitter.attr.ee, which now points here. Version 1 finds version 2 by itself at its next start. Click Yes in the update message. The update keeps the `profiles` folder and the PBs in it. Files that version 1 needs and version 2 does not, such as `Updater.exe` and the DLL files, stay in the folder. You can delete them.

### Make sure that a download is genuine

`SHA256SUMS` is signed with the AutoSplitter release key ([`packaging/minisign.pub`](packaging/minisign.pub)). Every file also has a GitHub build attestation that ties it to the workflow run that built it:

```
minisign -Vm SHA256SUMS -p minisign.pub       # the list is genuine
sha256sum -c --ignore-missing SHA256SUMS      # the files match it
gh attestation verify <file> --repo KimP0ssible-no/AutoSplitter
```

### New versions

At start, AutoSplitter reads the newest version from `https://kimp0ssible-no.github.io/AutoSplitter/version.json` on GitHub Pages, one request that sends nothing about you or your runs. If a newer version exists, AutoSplitter shows what changed and asks before it installs anything. It installs a download only if the signed `SHA256SUMS` of that release lists it with the same checksum.

## Set up a run

1. Select a profile, or click New to make one.
2. Click Set Region. Drag a rectangle around the place where the game shows its objective text, then press S or Enter.
3. If you use LiveSplit, add the LiveSplit Server component to your layout, then click Control > Start Server in LiveSplit. Select LiveSplit in AutoSplitter.
4. Click Save Settings.
5. Click Start (F9), then play. The timer starts when the start trigger text shows.

If LiveSplit compares against Game Time, it shows the times AutoSplitter measured from the matching frames.

### Keys

These keys work while the game has focus:

| Key | Action |
| --- | --- |
| F9 | Start |
| F10 | Reset |
| F11 | Stop |
| Middle mouse button | Split by hand. Before the start trigger shows, it starts the run. Click the Split key button in the settings to choose another key or mouse button. |

After the split key, OCR splits wait for the profile's cooldown (5 seconds by default).

To move the overlay, drag it. To resize it, drag the corner mark that shows in its bottom-right corner while the mouse is over the overlay. Click Save Settings to keep the position and size in the profile.

## Profiles

A profile is an XML file. Edit it with the Profile Editor (the Edit button) or in a text editor. This is a short example:

```xml
<SplitProfile profileVersion="2">
  <Game>The Division 2</Game>
  <Mission>Paradise Lost</Mission>
  <Capture targetFps="5" />
  <Detection cooldownSeconds="5" pbProtectionSeconds="120">
    <Pixel enabled="true" fadeThreshold="12" fadeFramesRequired="5" />
    <OCR enabled="true" region="441,60,996,140" intervalSeconds="1" changeThreshold="0.6"
         fuzzyMatchThreshold="0.92" ocrRetryFrames="3" language="en" />
  </Detection>
  <StartTrigger>
    <From>heroic</From>
    <To>enter the estate</To>
  </StartTrigger>
  <Segments>
    <Segment name="Courtyard to door" pb="44.800">
      <Keyword>find a way into the courtyard</Keyword>
    </Segment>
    <Segment name="Lovebirds" final="true" pte="240">
      <Keyword>paradise lost</Keyword>
    </Segment>
  </Segments>
  <LiveSplit enabled="false" host="localhost" port="16834" />
  <Overlay enabled="true" x="1" y="295" opacity="80" scale="1" />
  <Hotkeys manualSplitKey="Mouse3" />
</SplitProfile>
```

- `region` is left, top, right and bottom in screen pixels. The region can be on any monitor.
- The start trigger has two steps: a `From` text, then a `To` text in a later read. If there is no `From`, the first `To` text starts the run.
- A split can skip segments: the first later segment whose keyword shows is the split, and LiveSplit gets one skip for each skipped segment.
- `pte` is the number of seconds into the run before a segment can match. Use it when a keyword can also show early in the run.
- `fuzzyMatchThreshold` is how close the OCR text must be to a keyword, from 0.5 to 1.0.
- `ocrRetryFrames` is how many recent frames OCR tries, newest first, before it gives up on a read.
- `pbProtectionSeconds`: AutoSplitter does not offer to save a PB that is this much faster than the old one, because such a run is usually a mistake.
- AutoSplitter reads `<Pixel>` and keeps it, but does not use it.

## Build from source

You need Rust (stable, from [rustup.rs](https://rustup.rs)) and Visual Studio's C++ build tools on Windows. On Linux, the core and the OCR build and test too, but screen capture and the global keys need Windows.

1. Download the OCR models into `models/`. This needs `sh`, `curl` and `sha256sum`, for example from Git for Windows.

   ```
   sh packaging/fetch-models.sh
   ```

2. Build and test:

   ```
   cargo build --release
   cargo test --release
   ```

The programs are `target/release/autosplitter.exe` and `target/release/profile-editor.exe`. The packages name them `AutoSplitter.exe` and `ProfileEditor.exe`. `tests/ocr.rs` reads every image in `tests/images/` and fails if accuracy drops below that of version 1: 88% for English and 100% for Chinese.

## Code signing policy

Windows releases are signed with a [Certum](https://www.certum.eu) Open Source Code Signing certificate issued to the project's maintainer. This covers the programs, the setup and its uninstaller. The key stays in Certum's SimplySign cloud.

Only Windows files that the GitHub Actions CI workflow of this repository builds from its source are signed. This covers releases and test builds that the maintainer starts by hand. A person approves every run that signs files.

| Role | Members |
| --- | --- |
| Committers and reviewers | [Kim Attree](https://github.com/KimP0ssible-no) |
| Approvers | [Kim Attree](https://github.com/KimP0ssible-no) |

A committer reviews pull requests from anyone else before they are merged.

### Privacy policy

This program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it. There are two exceptions. The update check described under [New versions](#new-versions) is a single request to GitHub Pages that sends nothing about you or your runs. GitHub handles that request under the [GitHub General Privacy Statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement). If you select LiveSplit, AutoSplitter sends timer commands to the LiveSplit Server address in your profile. Screen images and OCR text stay on your computer.

## Credits

AutoSplitter is made by [@KimP0ssible](https://github.com/KimP0ssible-no). Thanks to @Summy_the_Dummy, @Bazookas and @Gast for their help with version 1, and to @Copeium for the fix in 1.3.3.

## Licence

Copyright 2026 Kim Attree. The source code is licensed under the Apache License, Version 2.0 (`LICENSE`). Redistributions must keep the `NOTICE` file. The user interface uses Slint under GPL-3.0-only (`SLINT-LICENSE.md`), so the programs as built and shipped are GPL-3.0-only. `NOTICE` lists the other parts the programs include: ONNX Runtime, the PaddleOCR models and the Visual C++ runtime.

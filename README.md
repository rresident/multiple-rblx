# Multiple Roblox

Multiple Roblox lets you run several Roblox instances on different accounts at the same time.

## Getting started

Grab the latest exe from [Releases](../../releases) and run it.

You need Windows 10 or 11, Roblox installed, and the Microsoft Edge WebView2
runtime. WebView2 ships with Windows 11 and recent Windows 10, so you probably
already have it. If sign-in refuses to open, install it from Microsoft and try
again.

Then:

1. Click **Add account** and sign in. Repeat for each account.
2. Turn on **Enable multiple Roblox**. Do this before starting any client.
3. Click the play button on a row. Pick a game. It launches.

If you want it always on, open Settings and turn on **Start with Windows**
and **Start hidden**. Your PC then boots into a state where every account can
launch, without a window in your face.

## Building it yourself

```
cargo build --release
```

Rust 1.90 or newer, and a Windows target. The binary lands in
`target/release/multiple-rblx.exe`.

Tests:

```
cargo test
```

Some tests are marked `#[ignore]` because they talk to the live Roblox API or
to Windows Credential Manager. Run those with `cargo test -- --ignored` if you
want them.

## Q&A

**Is this against the rules?**

Roblox does not support running more than one client and has worked to prevent
it. Plenty of tools have done this for years and it is widely treated as
tolerated rather than blessed. It is your account and your decision.

**Where do my cookies go?**

Into Windows Credential Manager, encrypted by Windows under your user account.
They never touch a file this app writes. Be aware that this protects them from
other Windows users, not from programs running as you.

**Why does it need a browser window to sign in?**

Because that way your password only ever goes into roblox.com. The window is
InPrivate, uses a throwaway profile that gets deleted afterwards, blocks
downloads and popups, and refuses to navigate anywhere outside roblox.com. All
this app keeps is the session cookie Roblox hands back.

**Does it work with Bloxstrap or Fishstrap?**

Yes.

**It says Roblox is already running and I do not think it is.**

Roblox leaves background processes behind after you play. If no actual game
client is running, the app clears them for you automatically. If one really is
running, close it first. Nothing gets killed while you are playing.

**Can I get my data back off this thing?**

Settings has a "Your data" page. "Clear cached files" removes leftover sign-in
browser data and old logs. "Delete all my data" removes the saved sign-ins,
your favourites and settings, and every cached file, then closes the app.

## Licence

MIT. See [LICENSE](LICENSE).

Not affiliated with, endorsed by, or connected to Roblox Corporation.

# Plan: stable code signing so keychain grants persist

Status: planned, not started. Follow-up to the `/usr/bin/security` shell-out already
in `src-tauri/src/state/token_cache.rs`.

## Why

macOS grants keychain access per requesting binary. The grant stores the binary's
**designated requirement** (DR), and the DR depends on how the binary is signed:

| signing | designated requirement | grant survives a rebuild? |
|---|---|---|
| ad-hoc / linker-signed | literal `cdhash` of the code | no |
| self-signed certificate | certificate hash + signing identifier | yes |
| Developer ID | Apple anchor + team OU + signing identifier | yes |

Every local build of this app is ad-hoc signed, so its DR is a hash of its own code.
Changing one byte of Rust produces a different DR, which macOS treats as a different
application, so an "Always Allow" grant can never stay attached. That is why the login
password dialog kept coming back several times a day.

Verified on this machine (macOS 26.5.2, build 25F84):

- the login keychain does **not** auto-lock, so the prompt was never a locked keychain
- the item is a legacy file-based generic-password item in `login.keychain-db`
- Claude Code rewrites the item when it rotates the token (`mdat` changes), and this does
  **not** wipe the existing grant — `/usr/bin/security` kept reading without prompting
  afterwards
- a targeted `security find-generic-password` lookup answers in ~10ms

The shell-out sidesteps the problem rather than fixing it: `/usr/bin/security` is
Apple-signed with a stable DR, so the grant given to it holds. The app itself still has
no durable identity of its own.

## What this plan changes

Give the app a stable code signature so it can hold a keychain grant under its own
identity, and so the distributed build stops re-prompting after every update.

This machine already has `Developer ID Application: Diego Vieira (NJVK5PA7FJ)`. Signing
is currently wired only into CI (`.github/workflows/release.yml` passes
`APPLE_CERTIFICATE` / `APPLE_SIGNING_IDENTITY` / `APPLE_TEAM_ID` to `tauri-action`);
`tauri.conf.json` has no `bundle.macOS` section, so local builds get nothing.

### Steps

1. **Add macOS bundle signing config** to `src-tauri/tauri.conf.json` under
   `bundle.macOS`, with the signing identity read from an environment variable so CI and
   local builds share one path and no identity is hardcoded in a public repo.

2. **Sign local dev builds.** `tauri dev` runs the raw Mach-O out of `target/debug/`,
   with no bundle. Either sign that binary after each build, or accept a self-signed
   certificate created once in Keychain Access and referenced from a build script. A
   self-signed cert is enough: its DR pins the certificate, not the code, so it is stable
   across rebuilds and needs no paid account.

3. **Pin the signing identifier.** A stable DR needs a stable identifier as well as a
   stable certificate; let Tauri set it from the bundle identifier rather than defaulting
   to the file name.

4. **Verify the DR is stable across a rebuild.** This is the acceptance test:

   ```
   codesign -d -r- <binary>   # before
   touch src-tauri/src/main.rs && cargo build
   codesign -d -r- <binary>   # after — must be byte-identical, and must not contain cdhash
   ```

5. **Only then** consider reverting the shell-out back to an in-process Keychain call.
   Optional: the subprocess costs ~25ms on a cache miss and reads are already throttled,
   so there is no performance reason to revert. Revert only if owning the identity is
   worth more than the simplicity of the current code.

### Non-goals

- Notarization. Separate concern, unrelated to keychain prompts.
- Sandboxing. Would change keychain access rules and is not needed here.
- Copying the token into an app-owned keychain item or a plaintext file. A file would be
  a real security regression, and an app-owned keychain item hits the same unstable-DR
  problem until this plan lands.

## Open question

An Apple Development certificate is the weakest of the stable options: its DR pins
`leaf[subject.CN]`, and Xcode-managed development certs get rotated and reissued, which
breaks the grant again. Prefer the Developer ID cert already on this machine, or a
self-signed cert that is never rotated.

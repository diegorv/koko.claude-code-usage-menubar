# Plan: stable code signing so keychain grants persist

Status: local builds done and verified. CI still unsigned — see "Remaining work".
Follow-up to the `/usr/bin/security` shell-out in `src-tauri/src/state/token_cache.rs`.

## Done

Local release builds sign with the Developer ID already on the machine, and need no
config change — the Tauri bundler picks the identity up from the environment:

```bash
export APPLE_SIGNING_IDENTITY="Developer ID Application: Diego Vieira (NJVK5PA7FJ)"
pnpm tauri build
```

`src-tauri/tauri.conf.json` deliberately keeps no `bundle.macOS.signingIdentity`: it
would hardcode one developer's identity into a public repo and break every other
build. The environment variable is the same channel CI uses.

The signed app carries a designated requirement with **no cdhash**:

```
designated => identifier "com.diegorv.claude-code-usage-menubar"
  and anchor apple generic
  and certificate 1[field.1.2.840.113635.100.6.2.6]
  and certificate leaf[field.1.2.840.113635.100.6.1.13]
  and certificate leaf[subject.OU] = NJVK5PA7FJ
```

Acceptance test from step 4 below, run against a real rebuild: the binary's cdhash
changed, and the designated requirement stayed byte-identical. A keychain grant given
to this app therefore survives rebuilds.

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

## Remaining work

**CI does not sign.** `.github/workflows/release.yml` passes `APPLE_CERTIFICATE`,
`APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` and `APPLE_TEAM_ID` to
`tauri-action`, but `gh secret list` returns nothing — none of them are set, so every
artifact a tag produces is ad-hoc signed. No release has been published yet, so this
has never actually bitten; the first tag will produce an unsigned build unless the
secrets land first.

To fix, export the Developer ID certificate **with its private key** as a `.p12` from
Keychain Access, then set four repository secrets:

| secret | value |
| --- | --- |
| `APPLE_CERTIFICATE` | `base64 -i cert.p12` output |
| `APPLE_CERTIFICATE_PASSWORD` | the password used for the `.p12` export |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: Diego Vieira (NJVK5PA7FJ)` |
| `APPLE_TEAM_ID` | `NJVK5PA7FJ` |

The `.p12` contains a private key. Export it, upload it, delete the local file; never
commit it, and never paste it into a terminal that logs.

### Original steps, for reference

1. ~~**Add macOS bundle signing config**~~ — not needed. The bundler reads
   `APPLE_SIGNING_IDENTITY` from the environment, so no config change was required.

2. **Sign local dev builds.** `tauri dev` runs the raw Mach-O out of `target/debug/`,
   with no bundle, so it stays ad-hoc. Left undone on purpose: since the app reads the
   keychain through `/usr/bin/security`, a dev build no longer needs a grant of its own,
   which was the only reason to sign it.

3. ~~**Pin the signing identifier.**~~ Already correct — the identifier comes out as
   `com.diegorv.claude-code-usage-menubar`, matching the bundle identifier.

4. **Verify the DR is stable across a rebuild.** This is the acceptance test — passed,
   see "Done" above:

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

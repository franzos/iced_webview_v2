# Security Policy

`iced_webview` embeds web-rendering engines (litehtml, Servo, CEF) into iced
applications, so it can end up parsing untrusted HTML and network content. I take
reports about it seriously, and thanks for taking the time to look.

## Supported versions

This is pre-1.0 software and moves fast. Only the latest tagged release and the
`main` branch get security fixes. There are no backports to older `0.x` tags, so
if you're running an older build, the fix is to upgrade.

| Version        | Supported |
| -------------- | --------- |
| latest release | yes       |
| `main`         | yes       |
| older `0.x`    | no        |

## Reporting a vulnerability

Please report privately, not through a public issue or pull request.

- Email: mail@gofranz.com
- If you use GitHub, you can also open a private advisory via the repository's
  Security tab ("Report a vulnerability").

Useful things to include, as far as you have them:

- what the issue is and the impact you think it has
- the affected version, commit, or engine backend (litehtml, Servo, CEF)
- steps to reproduce, or a proof of concept
- any logs or config (with secrets redacted) that help me confirm it

## What to expect

I'll acknowledge your report, confirm whether I can reproduce it, and keep you
updated as I work on a fix. Once it's resolved I'm happy to credit you in the
release notes, or keep you anonymous if you'd rather. Please give me a chance to
ship a fix before disclosing publicly.

## Scope

The `iced_webview` code in this repository is in scope. The embedded rendering
engines (litehtml, Servo, CEF) and other third-party dependencies are better
reported upstream, though I do want to hear about it if such an issue is
exploitable through how `iced_webview` uses it.

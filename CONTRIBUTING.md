# Contributing to Darwaza

Thank you for wanting to help. A few rules keep the project small and safe.

## Before you start
- **Open an issue first** for anything bigger than a typo or a one-line fix, so we can
  agree on the direction before you spend time on it.
- Security problems are **not** issues — see [SECURITY.md](SECURITY.md).

## What we may decline
Darwaza is deliberately small. We often say no to:
- features that need an account or a network connection to work (the client must stay
  fully usable offline and without an account);
- anything that weakens how secrets are stored or how the AI-assistant channel asks you
  to confirm changes;
- new dependencies for something a few lines can do;
- large visual redesigns without a prior discussion.

## Making a change
1. Fork, branch from `main`, keep the change focused (one topic per pull request).
2. Run the checks from the [README](README.md#checks): `pnpm lint`, `vue-tsc`,
   `scripts/check-locales.py`, the Playwright tests and `cargo test --lib`.
3. **Add a test that fails without your fix.** A test that passes either way proves nothing.
4. **Six languages together:** every new string goes into all of `src/locales/*.ts`
   (uz, ru, en, kk, ky, tr); the key counts must match.
5. **Changelog in the same commit:** describe what it gives the user, in
   `CHANGELOG.md` (English), `CHANGELOG_ru.md` and `CHANGELOG_uz.md` under the
   *Unreleased* section. If you cannot write Russian or Uzbek, write the English entry and
   say so in the pull request; we will translate it.
6. Do not edit an applied database migration — not even a comment. Add a new one.
7. No secrets, keys or real hostnames in commits, tests or screenshots.

## License of your contribution
By submitting a change you agree it is licensed under **GPL-3.0-or-later**, the license
of the project. Please sign off your commits (`git commit -s`) to certify the
[Developer Certificate of Origin](https://developercertificate.org/).

<!-- TODO (owner): decide before the first outside contribution whether to require a CLA
     instead of the DCO (needed only if closed-source licensing to companies is wanted). -->

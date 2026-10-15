# Security policy

## Reporting a vulnerability

Please **don't open a public issue** for security problems. Report them privately through
GitHub: open the repository's **Security** tab and choose **Report a vulnerability**
([direct link](https://github.com/IGUNUBLUE/mira/security/advisories/new)).

Include what you found, how to reproduce it, and its impact. You can expect a first reply
within a week. Once a fix is released, you will be credited in the advisory unless you
prefer not to be.

## Supported versions

Only the latest release gets security fixes.

## Scope

Especially relevant areas:

- handling of ChatGPT tokens and API keys (`src/openai.rs`), and anything that could leak
  them, for example through process arguments or file permissions;
- what is sent over the network when generating commit messages (see
  [docs/privacy.md](docs/privacy.md));
- terminal escape sequences that could make mira run commands or read data;
- the install script (`install.sh`).

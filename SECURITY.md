# Security policy

## Supported versions

Only the newest release of AutoSplitter gets security fixes. AutoSplitter looks for a new release at each start and offers to install it.

## Report a vulnerability

Do not report a vulnerability in a public issue.

1. Open the repository's Security tab on GitHub.
2. Click "Report a vulnerability".
3. Describe the problem, the affected version, and the steps that show it.

If you cannot use GitHub, send the report by email to autosplitter@attr.ee.

You get an answer to every report. When a fix is ready, it comes out in a new release, and the release notes name the problem. If you want, the advisory also names you.

## Scope

The policy covers the `AutoSplitter` and `ProfileEditor` programs, the setup and the portable zip on the release page, and the update path from a release to the next one.

Every night, `cargo deny` compares the Rust crates that AutoSplitter uses with the RustSec advisory database. A known advisory in a dependency does not need a report, but a report that shows how it affects AutoSplitter is welcome.

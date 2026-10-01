# Security policy

xentop-ng runs as root in dom0, so we take bugs that let less-privileged
parties influence it seriously. Examples:
- a VM name, guest-driven counters or files in `/dev/shm` that crash it,
  spoof its display or inject terminal escape sequences;
- library-loading issues;
- anything in the libxenstat patches.

**Please don't open a public issue for a vulnerability.** Report it
privately through GitHub's
[private vulnerability reporting](https://github.com/olivierlambert/xentop-ng/security/advisories/new).
We'll acknowledge it within a few working days.

Security issues in Xen itself, including the upstream parts of libxenstat,
should go to the [Xen Project security team](https://xenproject.org/developers/security-policy/).

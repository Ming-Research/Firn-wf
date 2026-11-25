# The Firn constitution

This constitution holds what Firn is for and its lasting values, and every
decision in the project comes from them. The owner sets it and changes it.

## What Firn is for

Firn is a server of Redis's protocol written in Whitefoot. It exists first for
Whitefoot: it is the real server that shows what Whitefoot gives a concurrent,
persistent network program, safety from the source together with highly
concurrent computation and I/O, and it exposes what Whitefoot still lacks. It
shows three things together, and without any one of them the showing fails: it
is far faster than every existing server of the protocol; people can use it in
place of Redis in production; and others can check its safety. Second, it may
become a product in its own right.

## Article 1 (usable). Firn can take Redis's place in production

Existing applications use Firn in place of Redis in production, through their
ordinary clients and with their logic unchanged, and the data they give it is
safe with it.

Violations: an application Firn sets out to serve that cannot use it in place of
Redis; data Firn accepted and then lost.

## Article 2 (safe). Safety from Whitefoot's proofs, checkable by others

Firn's safety comes from Whitefoot's proofs: it keeps every check Whitefoot
requires, and nothing turns one off. Others can check what it guarantees.

Violations: an unchecked operation or an escape from Whitefoot's checks, for any
reason; a guarantee others cannot check.

## Article 3 (fast). Far faster than every rival

On the same machine, Firn is far faster than the fastest existing server of the
protocol, and its throughput grows with the machine's cores. Speed counts only
on top of Articles 1 and 2.

Violations: a speed-up that breaks Article 1 or 2.

## Article 4 (for Whitefoot). A gap goes back to Whitefoot

What Firn cannot write well or make fast in Whitefoot is a gap of Whitefoot,
closed in Whitefoot; a fault of Firn's own is fixed in Firn.

Violations: a gap of Whitefoot that Whitefoot never heard of.

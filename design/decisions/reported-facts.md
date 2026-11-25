# Reported facts

What firn says about itself in Redis's form, as INFO and CLIENT INFO do.

### decision reported-facts-real-fixed-or-absent
Where firn reports facts about itself in Redis's form, a field it has carries its real value, a field fixed for firn carries that value, and a field it cannot measure is left out.
- rejected: an agent proposed placeholder values for the fields firn cannot measure, such as addr=?:0 or used_memory:0; the owner rejected it, because nothing tells a reader them from measurements, and a client or an operator could act on them.
- rejected: an agent proposed refusing INFO and CLIENT INFO until firn measures every field; the owner rejected it, because Django's test sends CLIENT INFO and fails on the refusal, while it reads no field firn leaves out (data: `research/investigations/consumers/README.md`).

### decision info-firn-diagnostics-after-redis-fields
INFO lists firn's own append-only rewrite diagnostics in the Persistence section, after every Redis 7.2.16 field of that section.
- rejected: an agent proposed placing firn's own fields among Redis's fields; the owner rejected it, because a separate tail keeps Redis's fields in their relative order.
- rejected: an agent proposed deleting those diagnostics; the owner rejected it, because operators and the rewrite acceptance cases read their real measurements.

### decision client-list-other-connections-show-only-the-facts-they-set
CLIENT LIST gives the caller every CLIENT INFO field and each other connection only the facts it sets itself, with its subscription counts read from the event hub.
- rejected: an agent proposed publishing idle, flags, multi and cmd to the client table on every command; the owner rejected it, because that is a shared write on every command of every connection, and replication and MONITOR, which the suite's flags checks need, are outside firn's features.

### decision unreported-address-is-empty
A connection's address that the host refuses to report is shown empty, as addr= or laddr= with nothing after the equals sign, as Redis 7.2.16's genClientAddrString does.
- rejected: an agent proposed the placeholder ?:0 that genClientAddrString's comment promises; the owner rejected it, because decision reported-facts-real-fixed-or-absent allows no placeholder, which a reader cannot tell from a measurement.

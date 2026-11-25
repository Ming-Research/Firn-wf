# Protocol

Which versions of Redis's protocol firn answers, and how each reply takes its type.

### decision resp2-and-resp3-in-shared-writers
firn answers RESP2 and RESP3: a connection keeps the version HELLO last chose, and the shared reply writers take each reply's type from it.
- replaced: refusing HELLO 3 was on main; the owner changed it, because two of the three selected consumers' clients, redis-py 8 and node-redis 6, open every connection with HELLO 3 and stop when it is refused (data: `research/investigations/consumers/README.md`).
- rejected: an agent proposed a reply writer per protocol, each command choosing one; the owner rejected it, because each command would write its reply twice, while only the type of a few elements differs, which Redis decides in its shared writers.
- rejected: an agent proposed translating RESP2 replies into RESP3 after a command writes them; the owner rejected it, because a RESP2 reply has lost the types RESP3 needs: a map and an array of pairs, and a double and a bulk string, are written alike.
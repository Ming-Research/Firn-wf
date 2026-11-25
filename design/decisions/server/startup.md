# Startup

How firn is started and configured: its arguments, the configuration file and its paths, the directives, the bind address, the client limit and the file-descriptor limit.

### decision startup-follows-redis-server
firn takes an optional Redis 7.2.16 configuration file or stdin -, then --directive arguments applied after the file, and refuses any directive it does not know.
- replaced: four positional arguments (port, client count, AOF name and idle timeout) was on main; the owner changed it, because an operator replacing redis-server could not reuse its start command or config file, and firn read the config path as a port.
- rejected: an agent proposed reading the file but ignoring unknown directives with a warning; the owner rejected it, because an ignored directive such as save or appendfsync always would let the operator believe a persistence setting was in force.

### decision config-paths-stay-in-the-working-directory
Until Whitefoot can open general paths, firn opens a configuration file only relative to its working directory, accepts only . and ./ for dir, and refuses include.
- rejected: an agent proposed waiting for Whitefoot's path support and delivering startup in one step; the owner rejected it, because configuration, log levels and maxclients, which need no paths, would then wait on Whitefoot too.

### decision startup-accepts-only-implemented-directives
Startup accepts only the directives firn implements, and checks the memory settings, maxclients, port and timeout with CONFIG SET's own validation code.
- rejected: an agent proposed silently accepting unsupported Redis directives when they hold their default values; the owner rejected it, because refusing them shows the compatibility still to be built.
- replaced: separate validation of port and timeout at startup and in CONFIG SET was on main; the owner changed it, because both entry points must accept the same values and give the same reasons.

### decision bind-takes-one-ipv4-address
bind takes one dotted IPv4 address or *, refuses several addresses, and an empty bind opens no listener, as in Redis.
- rejected: an agent proposed accepting several bind addresses and listening on one; the owner rejected it, because accepting them would promise listeners that firn does not open.
- rejected: an agent proposed doing bind in full first, with several addresses, IPv6 and a listener for each; the owner rejected it, because that rework of firn's listener would take a day or two more before startup could merge.

### decision a-startup-directive-bounds-the-test-server
A test directive of firn's own makes firn accept N clients and then stop, with zero meaning no limit.
- replaced: the client count as a positional argument was on main; the owner changed it, because tests need a bounded server lifetime, and a directive of firn's own keeps that apart from Redis's deployment settings.

### decision bare-arguments-before-a-directive-are-refused
firn refuses a bare argument after the configuration filename and before the first --, even where Redis would join the arguments into a valid directive.
- rejected: an agent proposed building a configuration line from bare arguments, as Redis does for firn redis.conf timeout 3; the owner rejected it, because a misplaced extra argument or an old positional argument should be an error, not become a directive.

### decision maxclients-follows-redis
maxclients works as in Redis 7.2.16: default 10000, range 1 to 4294967295, a refused peer gets -ERR max number of clients reached, and only rejected_connections counts it.
- rejected: an agent proposed adding maxclients only once Whitefoot could raise the descriptor limit, doing both together; the owner rejected it, because firn would have no client limit at all until then.

### decision client-admission-reads-maxclients-in-one-statement
Admission reads maxclients and counts the new client in one atomic statement, and a refused peer's reply gets one shared one-second send deadline before firn closes it.
- rejected: an agent proposed waiting until a refused peer takes the whole reply; the owner rejected it, because a peer that does not read would hold firn's single accept loop without bound.

### decision descriptor-limit-left-unchanged
firn leaves the operating system's file-descriptor limit unchanged and keeps maxclients as configured, at startup and on CONFIG SET.
- rejected: an agent proposed lowering or refusing maxclients by a computed estimate of the descriptors available; the owner rejected it, because firn promises an application client limit, not descriptor availability, and the Whitefoot release then had no way to read the limit.

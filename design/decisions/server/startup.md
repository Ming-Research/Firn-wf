# Startup

How firn is started and configured: its arguments, the configuration file and its paths, the directives, the bind address, the client limit and the file-descriptor limit.

### decision startup-follows-redis-server
firn takes an optional Redis 7.2.16 configuration file or stdin -, then --directive arguments applied after the file, and refuses any directive it does not know.
- replaced: four positional arguments (port, client count, AOF name and idle timeout) was on main; the owner changed it, because an operator replacing redis-server could not reuse its start command or config file, and firn read the config path as a port.
- rejected: an agent proposed reading the file but ignoring unknown directives with a warning; the owner rejected it, because an ignored directive such as save or appendfsync always would let the operator believe a persistence setting was in force.

### decision config-paths-are-host-paths
firn opens the configuration file, each include and each dir as a host path, absolute or relative to the working directory; dir changes that directory by swapping the held handles.
- replaced: opening a configuration file only relative to the working directory, accepting only . and ./ for dir and refusing include was on main; the owner changed it, because Whitefoot now opens host paths, and an operator replacing redis-server could not start firn with /etc/redis/redis.conf.

### decision include-refuses-wildcards
An include path holding *, ? or [ is refused in the fatal configuration frame, naming wildcards; any other include is read where the directive stands, with line numbers of its own.
- rejected: an agent proposed expanding wildcards in the last path component, sorted, and skipping a pattern that matches nothing; the owner rejected it, because it needs a directory listing and a rewrite of glob(3)'s details, about a day more, and character classes would still differ from Redis.
- rejected: an agent proposed a full glob(3) in every path component; the owner rejected it, because it is the most work and no selected consumer needs it.

### decision dir-reports-the-written-path
CONFIG GET dir reports the absolute path the dir directives wrote, with . .. and repeated / removed, and reports nothing while every dir given was relative to a directory firn does not know.
- rejected: an agent proposed leaving CONFIG GET dir unsupported until Whitefoot can query the working directory's path; the owner rejected it, because a configuration with an absolute dir would then not read back as Redis reads it, which the work's result requires.

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

### decision maxclients-adjusts-the-descriptor-limit
At startup and on CONFIG SET maxclients, firn raises the soft open-files limit to maxclients plus 32 as Redis does, lowers maxclients if refused, and refuses a CONFIG SET it cannot support.
- replaced: leaving the operating system's file-descriptor limit unchanged was on main; the owner changed it, because Whitefoot now reads and raises the soft limit, and an operator had to set ulimit -n before every start.
- rejected: an agent proposed adjusting the limit only at startup and letting CONFIG SET maxclients accept any value; the owner rejected it, because a limit raised while firn runs could then exceed the descriptors, where Redis refuses it.

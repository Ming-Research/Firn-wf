# Script replies and number text

How the replies of a script's commands reach Lua, how a script's errors are written, and the text firn writes for numbers.

### decision script-command-replies-pass-through-resp-bytes
A command a script calls writes its reply into the client's buffer as on the network, and the script gets it converted from those bytes by Redis's redisProtocolToLuaType rules.
- rejected: an agent proposed a second, Lua-specific result for every command; the owner rejected it, because from the bytes a script's command runs the network path's own reply writers.

### decision script-raised-error-written-on-one-line
A raised script error is written as Redis 7.2.13 and later write it, each carriage return and line feed of its text as a space.
- replaced: Writing a raised error's text raw, as Redis 7.0.15 does, was on main; the owner changed it, because no client reads such a reply as one error, so it keeps no compatibility and leaves the reply splitting Redis 7.2.13 fixed as a security defect.

### decision scores-module-writes-redis-number-text
The scores module writes every number text firn shares with Redis 7.2.16 by the rule Redis picks for that command and encoding, with fpconv_dtoa's digits from Halo's port.
- replaced: %.17g for script numbers and command-reply scores was on main; the owner changed it, because Redis 7.2.16 writes them with d2string and fpconv_dtoa, and a client sees 0.10000000000000001 where Redis answers 0.1.
- rejected: an agent proposed an exact shortest formatter; the owner rejected it, because Redis 7.2.16's Grisu2 has no exact fallback, and its digits differ from the exact shortest for 355 of 16,922 values redis-server 7.2.16 wrote.
- replaced: Separate number text code in scripting and in scores was on main; the owner changed it, because Redis 7.2.16 picks among the same few text rules by command and encoding, so one module holds them all.
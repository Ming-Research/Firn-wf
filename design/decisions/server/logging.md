# Logging

How firn writes its log: the line format, where the lines go, which events write them and what they say of the product.

### decision logfile-is-opened-again-for-every-line
With logfile set, firn opens the file for appending for each line and closes it again, so a file renamed away is created anew, as Redis does; a file whose last component is a symbolic link is refused.
- rejected: an agent proposed writing only to standard output and refusing logfile; the owner rejected it, because configurations that set logfile, including the distributions' own, could not start.
- rejected: an agent proposed opening the file once and holding it; the owner rejected it, because after a rotation by rename firn would go on writing the old file, where Redis writes the new one.

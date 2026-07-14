use strict;
use warnings;
use Storable qw(freeze);

open my $fh, ">", "specimen.bin" or die "can't write: $!";
binmode $fh;
print $fh freeze({ answer => 42 });
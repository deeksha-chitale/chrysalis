use strict;
use warnings;
use Sereal::Encoder qw(encode_sereal);

my $data = { answer => 42 };
open my $fh, ">", "sereal_specimen.bin" or die "can't write: $!";
binmode $fh;
print $fh encode_sereal($data);
close $fh;

print "wrote sereal_specimen.bin\n";
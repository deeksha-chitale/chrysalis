use strict;
use warnings;
use Storable qw(freeze);
use Scalar::Util qw(weaken);

# --- Build a data structure exercising every Storable feature ---

# Simple scalars
my $small_int = 42;                                  # SX_BYTE
my $big_int   = 100_000;                             # SX_INTEGER
my $float     = 3.14159;                             # SX_DOUBLE
my $bytes     = "\xFF\x00\xFE";                      # SX_SCALAR (raw bytes)
my $utf8      = "hello, world";                      # SX_UTF8STR

# A long string (over 256 chars) to hit SX_LSCALAR / SX_LUTF8STR
my $long = "x" x 300;

# Immortals
my $undef = undef;                                   # SX_UNDEF

# A blessed object
my $animal = bless { name => "Rex", legs => 4 }, "Animal";

# Another instance of the same class — hits SX_IX_BLESS on second use
my $animal2 = bless { name => "Fido", legs => 4 }, "Animal";

# Shared structure — two array slots point at the SAME hashref
my $shared = { count => 1 };
my $sharing_array = [ $shared, $shared ];

# A cycle — array whose element points back at the array
my $cyclic = [];
push @$cyclic, $cyclic;
weaken($cyclic->[0]);                                # SX_WEAKREF for the back-edge

# Regexp
my $regex = qr/^hello.*world$/i;

# Version string
my $vstring = v1.2.3;

# Nested array of hashes of arrays
my $nested = [
    { fruits => ["apple", "banana"] },
    { fruits => ["cherry"] },
];

# --- Bundle it all together ---
my $everything = {
    scalars => {
        small_int => $small_int,
        big_int   => $big_int,
        float     => $float,
        bytes     => $bytes,
        utf8      => $utf8,
        long      => $long,
        undef_val => $undef,
    },
    blessed => [ $animal, $animal2 ],
    shared  => $sharing_array,
    cyclic  => $cyclic,
    regex   => $regex,
    vstring => $vstring,
    nested  => $nested,
};

open my $fh, ">", "big_specimen.bin" or die "can't write: $!";
binmode $fh;
print $fh freeze($everything);
close $fh;

print "wrote big_specimen.bin ($everything)\n";
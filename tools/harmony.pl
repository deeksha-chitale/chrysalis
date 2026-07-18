use strict;
use warnings;
use Storable qw(freeze);
use Scalar::Util qw(weaken);
use Hash::Util qw(lock_keys);

# ============================================================
# SX_CODE requires $Storable::Deparse = 1
# SX_EVAL requires $Storable::Eval = 1
$Storable::Deparse = 1;
$Storable::Eval    = 1;

# ============================================================
# Package with STORABLE_freeze hook (SX_HOOK)
{
    package Point;
    sub new { bless { x => $_[1], y => $_[2] }, $_[0] }
    sub STORABLE_freeze {
        my ($self, $cloning) = @_;
        return ("$self->{x},$self->{y}");   # no extra refs
    }
    sub STORABLE_thaw {
        my ($self, $cloning, $data) = @_;
        ($self->{x}, $self->{y}) = split /,/, $data;
    }
}

# Package with operator overloading (SX_OVERLOAD, SX_WEAKOVERLOAD)
{
    package MyNum;
    use overload '+' => \&add, '""' => \&str, fallback => 1;
    sub new { bless { val => $_[1] }, $_[0] }
    sub add { MyNum->new($_[0]{val} + (ref $_[1] ? $_[1]{val} : $_[1])) }
    sub str { $_[0]{val} }
}

# Tied scalar (SX_TIED_SCALAR)
{
    package TieScalar;
    sub TIESCALAR { bless { v => $_[1] }, $_[0] }
    sub FETCH     { $_[0]{v} }
    sub STORE     { $_[0]{v} = $_[1] }
}

# Tied array (SX_TIED_ARRAY)
{
    package TieArray;
    sub TIEARRAY  { bless { d => [] }, $_[0] }
    sub FETCH     { $_[0]{d}[$_[1]] }
    sub STORE     { $_[0]{d}[$_[1]] = $_[2] }
    sub FETCHSIZE { scalar @{$_[0]{d}} }
    sub STORESIZE { }
    sub EXISTS    { exists $_[0]{d}[$_[1]] }
    sub DELETE    { delete $_[0]{d}[$_[1]] }
    sub PUSH      { my $self = shift; push @{$self->{d}}, @_ }
    sub POP       { pop @{$_[0]{d}} }
    sub SHIFT     { shift @{$_[0]{d}} }
    sub UNSHIFT   { my $self = shift; unshift @{$self->{d}}, @_ }
    sub SPLICE    { }
}

# Tied hash (SX_TIED_HASH)
{
    package TieHash;
    sub TIEHASH  { bless {}, $_[0] }
    sub FETCH    { $_[0]{$_[1]} }
    sub STORE    { $_[0]{$_[1]} = $_[2] }
    sub EXISTS   { exists $_[0]{$_[1]} }
    sub DELETE   { delete $_[0]{$_[1]} }
    sub CLEAR    { %{$_[0]} = () }
    sub FIRSTKEY { my $a = keys %{$_[0]}; each %{$_[0]} }
    sub NEXTKEY  { each %{$_[0]} }
}

# ============================================================
# SX_BYTE (0x08) — integers -128..127
my $byte_val = 42;

# SX_INTEGER (0x06) — larger native integer (8 bytes on 64-bit)
my $int_val = 100_000;

# SX_DOUBLE (0x07) — native float
my $dbl_val = 2.718281828;

# SX_UNDEF (0x05)
my $undef_val = undef;

# SX_SV_YES (0x0F) and SX_SV_NO (0x10)
my $yes_val = (1 == 1);
my $no_val  = (1 == 0);

# SX_SCALAR (0x0A) — byte string <= 255 bytes
my $bytes_val = "raw\xFF\x00bytes";

# SX_LSCALAR (0x01) — byte string > 255 bytes
my $lscalar_val = "B" x 300;

# SX_UTF8STR (0x17) — utf-8 string <= 255 bytes (non-ASCII forces flag)
my $utf8_val = "caf\x{e9}";

# SX_LUTF8STR (0x18) — utf-8 string > 255 bytes
my $lutf8_val = "\x{e9}" x 200;

# SX_VSTRING (0x1D) — version string
my $ver = v1.22.333;

# SX_REGEXP (0x20) — compiled regex
my $re = qr/^\d+\.\d+$/;

# SX_CODE (0x1A) — code ref (deparsed)
my $code = sub { my ($x, $y) = @_; $x + $y };

# ============================================================
# SX_ARRAY (0x02) — array containing mixed types
my @mixed_arr = (
    $byte_val,              # SX_BYTE
    $int_val,               # SX_INTEGER
    $dbl_val,               # SX_DOUBLE
    $undef_val,             # SX_UNDEF
    $yes_val,               # SX_SV_YES
    $no_val,                # SX_SV_NO
    $bytes_val,             # SX_SCALAR
    $utf8_val,              # SX_UTF8STR
    $re,                    # SX_REGEXP (inside array)
);

# SX_HASH (0x03) — hash with various value types
my %info_hash = (
    name    => "chrysalis",         # SX_SCALAR
    version => $ver,                # SX_VSTRING
    count   => $int_val,            # SX_INTEGER
    pi      => $dbl_val,            # SX_DOUBLE
    active  => $yes_val,            # SX_SV_YES
    deleted => $no_val,             # SX_SV_NO
    data    => $bytes_val,          # SX_SCALAR
    label   => $utf8_val,           # SX_UTF8STR
);

# SX_REF (0x04) — reference to a scalar
my $scalar_ref = \$int_val;

# SX_BLESS (0x11) — first use of "Animal" class
my $dog = bless {
    name   => "Rex",                # SX_SCALAR inside blessed hash
    legs   => 4,                    # SX_BYTE
    tricks => ["sit", "stay"],      # SX_ARRAY of SX_SCALAR inside blessed hash
}, "Animal";

# SX_IX_BLESS (0x12) — second use of same class (index form)
my $cat = bless {
    name  => "Whiskers",
    legs  => 4,
    sound => "meow",
}, "Animal";

# SX_HOOK (0x13) — object with STORABLE_freeze
my $point = Point->new(3, 4);

# SX_OVERLOAD (0x14) — overloaded object via ref
my $num_a = MyNum->new(10);
my $num_b = MyNum->new(20);

# SX_FLAG_HASH (0x19) — restricted hash
my %config = (host => "localhost", port => "5432");
lock_keys(%config);

# SX_OBJECT (0x00) — shared structure (two refs to the same thing)
my $shared_node = { id => 99, label => "shared" };
my @graph = ($shared_node, $shared_node);   # SX_OBJECT on second ref

# SX_WEAKREF (0x1B) — cycle broken with weak ref
my $node_a = { name => "a" };
my $node_b = { name => "b", peer => $node_a };
$node_a->{peer} = $node_b;
weaken($node_a->{peer});                    # SX_WEAKREF here

# SX_WEAKOVERLOAD (0x1C) — weak ref to overloaded object
my $num_c        = MyNum->new(99);
my $weak_container = [$num_c];
weaken($weak_container->[0]);

# SX_TIED_SCALAR (0x0D)
my $tied_s;
tie $tied_s, 'TieScalar', "tied_value";

# SX_TIED_ARRAY (0x0B)
my @tied_a;
tie @tied_a, 'TieArray';
push @tied_a, "hello", "tied", "world";

# SX_TIED_HASH (0x0C)
my %tied_h;
tie %tied_h, 'TieHash';
$tied_h{greeting} = "hi";
$tied_h{count}    = 3;

# SX_SV_UNDEF (0x0E) / SX_SVUNDEF_ELEM (0x1F) — sparse array with explicit undefs
my @sparse = (1, undef, 3, undef, 5);

# SX_LSCALAR in a nested context
my $deep = {
    outer => {
        inner => {
            blob   => $lscalar_val,     # SX_LSCALAR deep inside hashes
            text   => $lutf8_val,       # SX_LUTF8STR
        }
    }
};

# ============================================================
# Final bundle — everything in one structure
my $harmony = {
    # Scalars
    byte_val    => $byte_val,
    int_val     => $int_val,
    dbl_val     => $dbl_val,
    undef_val   => $undef_val,
    yes_val     => $yes_val,
    no_val      => $no_val,
    bytes_val   => $bytes_val,
    lscalar_val => $lscalar_val,
    utf8_val    => $utf8_val,
    lutf8_val   => $lutf8_val,
    ver         => $ver,
    re          => $re,
    code        => $code,

    # Containers
    mixed_arr   => \@mixed_arr,
    info_hash   => \%info_hash,
    scalar_ref  => $scalar_ref,

    # Blessed
    dog         => $dog,
    cat         => $cat,

    # Hook
    point       => $point,

    # Overloaded
    num_a       => $num_a,
    num_b       => $num_b,

    # Flag hash
    config      => \%config,

    # Shared / cyclic
    graph       => \@graph,
    node_a      => $node_a,
    weak_ovld   => $weak_container,

    # Tied
    tied_s      => \$tied_s,
    tied_a      => \@tied_a,
    tied_h      => \%tied_h,

    # Sparse
    sparse      => \@sparse,

    # Deep nesting
    deep        => $deep,
};

open my $fh, ">", "harmony.bin" or die "can't write: $!";
binmode $fh;
print $fh freeze($harmony);
close $fh;

printf "wrote harmony.bin (%d bytes)\n", -s "harmony.bin";
printf "keys: %s\n", join(", ", sort keys %$harmony);
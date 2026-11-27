struct Bits {
    int low : 3;
    int high : 5;
} bits = {1, 2};

union Short {
    char c;
    long long wide;
} short_first = {1};

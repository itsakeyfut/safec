union Word {
    long long whole;
    char byte;
};

union Word global;
static union Word table[2];

long long read(void) {
    union Word local;
    local.whole = global.whole + table[1].whole;
    return local.whole;
}

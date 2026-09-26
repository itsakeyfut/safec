int zero(void) {
    __attribute__((annotate("safec_unchecked"))) int x = 0;
    return x;
}

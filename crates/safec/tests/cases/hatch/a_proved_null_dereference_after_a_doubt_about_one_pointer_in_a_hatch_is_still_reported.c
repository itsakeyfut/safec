__attribute__((annotate("safec_unchecked")))
int proof_first(int c, int *p) {
    return c ? (p = 0, *p) : *p;
}
__attribute__((annotate("safec_unchecked")))
int doubt_first(int c, int *p) {
    return c ? *p : (p = 0, *p);
}

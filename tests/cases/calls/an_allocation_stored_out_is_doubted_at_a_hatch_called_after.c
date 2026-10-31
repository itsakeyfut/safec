void *malloc(int n);
__attribute__((annotate("safec_unchecked")))
void nothing(void) {
}
void put(int ** _Nonnull pp) {
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    *pp = x;
    nothing();
}

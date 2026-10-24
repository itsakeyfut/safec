__attribute__((annotate("safec_unchecked")))
int raw(int *p) {
    return *p;
}

__attribute__((annotate("safec_unchecked")))
int quiet(void) {
    return 0;
}

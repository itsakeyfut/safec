__attribute__((annotate("safec_unchecked")))
int both(int c, int *p) {
    int *q = 0;
    return c ? *q : *p;
}

__attribute__((annotate("safec_unchecked")))
int both(int *p) {
    int *q = 0;
    return *p && *q;
}

void *malloc(int n);
void free(void *p);

__attribute__((annotate("safec_unchecked"))) int f(void) {
    int *p;
    if (p != 0) {
        *p = 1;
    }
    return 0;
}

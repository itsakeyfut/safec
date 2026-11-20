void *malloc(int n);
void free(void *p);
__attribute__((annotate("safec_unchecked"))) int f(int c) {
    int *p = malloc(4);
    int n;
    n = 0;
    free(p);
    if (c) goto inside;
    while (n < 3) {
        free(p);
    inside:
        n = n + 1;
    }
    return 0;
}

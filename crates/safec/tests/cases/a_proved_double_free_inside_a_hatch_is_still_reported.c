void *malloc(int n);
void free(void *p);

__attribute__((annotate("safec_unchecked")))
int twice(void) {
    int *p = malloc(4);
    free(p);
    free(p);
    return 0;
}

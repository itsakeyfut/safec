void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(8);
    int i = p;
    free(p);
    int *base = malloc(4);
    int *r = base + i;
    free(r);
    return 0;
}

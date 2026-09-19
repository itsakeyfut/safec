void *malloc(int n);
void free(void *p);
int f(int c, int i) {
    int *q = malloc(4);
    if (c) { q = malloc(8); }
    free(q);
    int *p = q + i;
    free(p);
    return 0;
}

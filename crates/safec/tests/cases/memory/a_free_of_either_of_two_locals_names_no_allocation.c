void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    int *q = malloc(8);
    int *r;
    if (c) { r = p; } else { r = q; }
    free(r);
    free(r);
    return 0;
}

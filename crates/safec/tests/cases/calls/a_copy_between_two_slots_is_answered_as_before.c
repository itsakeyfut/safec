void *malloc(int n);
void free(void *p);
int copy(int **dst, int **src) {
    if (!dst) return 0;
    if (!src) return 0;
    int *s = *src;
    if (!s) return 0;
    int *d = malloc(4);
    if (!d) return 0;
    *d = *s;
    *dst = d;
    return *s;
}

// #691: a string copy into zeroed memory may leave the whole pointer there,
// so the pointer read back is the freed `p`.
void *malloc(int n); void *calloc(int n, int s); void free(void *p);
char *strcpy(char *d, char *s);
int main(void) {
    int *p = malloc(4);
    if (!p) return 1;
    int **a = calloc(1, 8); int **b = calloc(1, 8);
    if (!a || !b) return 1;
    *a = p;
    void *va = a; void *vb = b;
    char *ca = va; char *cb = vb;
    strcpy(cb, ca);
    free(p);
    int *q = *b;
    if (!q) return 0;
    int r = *q;
    return r;
}

void free(void *p);
void *malloc(int n);
int f(int **pp, int **pp2) {
    if (!pp) return 0;
    if (!pp2) return 0;
    int *q = *pp;
    if (!q) return 0;
    int *s = *pp2;
    if (!s) return 0;
    int **box = malloc(8);
    if (!box) return 0;
    int ***bb = malloc(8);
    if (!bb) return 0;
    *box = q;
    *bb = box;
    free(**bb);
    return *s;
}

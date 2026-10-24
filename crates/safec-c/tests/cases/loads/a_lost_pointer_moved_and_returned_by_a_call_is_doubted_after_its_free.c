void *malloc(int n);
void free(void *p);
int *idp(int **pp);
int *idb(int **b);
int *id(int *p);
int f(int c) {
    int **box = malloc(8);
    if (!box) return 0;
    int *u = 0;
    int *q = 0;
    int i = 0;
    while (i < 2) { u = q; q = malloc(4); if (!q) return 0; *q = 1; i = i + 1; }
    int *r = id(u + 1);
    free(u);
    if (!r) return 0;
    int v = *r;
    return v;
}

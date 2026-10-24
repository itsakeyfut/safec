void *malloc(int n);
void free(void *p);
void get(int **out, int *p) { *out = p; }
void put(int **box, int *p) { *box = p; }
int *take(int **box) { return *box; }
int f(int c) { 
    int *u = 0; int *q = 0; int i = 0;
    while (i < 2) { u = q; q = malloc(4); if (!q) return 0; *q = 1; i = i + 1; }
    int *u2 = malloc(4); if (!u2) return 0; *u2 = 1;
    int *r = 0;
    get(&r, u2);
    int *s = r;
    free(u2);
    if (!s) return 0;
    int v = *s;
    free(q);
    return v;
}

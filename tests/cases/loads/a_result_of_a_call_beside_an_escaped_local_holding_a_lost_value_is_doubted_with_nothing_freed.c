void *malloc(int n);
void free(void *p);
void init(int **pp);
int *get(void);
int f(int c) {
    int *p = 0;
    init(&p);
    int *q = 0;
    int i = 0;
    while (i < 2) {
        p = q;
        q = malloc(4);
        if (!q) return 0;
        *q = 1;
        i = i + 1;
    }
    int *b = get();
    if (!b) return 0;
    int v = *b;
    free(q);
    return v;
}

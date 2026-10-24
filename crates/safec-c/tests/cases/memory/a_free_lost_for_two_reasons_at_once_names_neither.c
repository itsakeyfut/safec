void *malloc(int n);
void free(void *p);
void init(int **p);
int **table(void);
int f(int c) {
    int *p;
    init(&p);
    int **t = table();
    if (t == 0) {
        return 0;
    }
    int *r = p;
    if (c) {
        r = *t;
    }
    free(r);
    return 0;
}

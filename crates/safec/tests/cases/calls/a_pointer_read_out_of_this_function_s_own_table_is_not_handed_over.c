void *malloc(int n);
void free(void *p);
void release(void);
int f(void) {
    int **t = malloc(8);
    int *own = malloc(4);
    if (t == 0) {
        return 0;
    }
    if (own == 0) {
        return 0;
    }
    *t = own;
    int *q = *t;
    release();
    int v = *q;
    free(own);
    free(t);
    return v;
}

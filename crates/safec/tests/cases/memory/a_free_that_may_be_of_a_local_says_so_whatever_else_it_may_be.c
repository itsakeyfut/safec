void *malloc(int n);
void free(void *p);
int **table(void);
int f(int c) {
    int x = 0;
    int **t = table();
    if (t == 0) {
        return 0;
    }
    int *r = &x;
    if (c) {
        r = *t;
    }
    free(r);
    return 0;
}

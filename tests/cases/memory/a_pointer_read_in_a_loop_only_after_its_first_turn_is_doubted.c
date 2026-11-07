void *malloc(int n);
void free(void *p);
void abort(void);
int *find(int k);
int *next(void);
void use(int *p);

int f(int n, int *q) {
    int *p;
    int i;
    int r = 0;
    for (i = 0; i < n; i++) {
        if (i > 0) {
            r = r + (p == q);
        }
        p = find(i);
    }
    return r;
}

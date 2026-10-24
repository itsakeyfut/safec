void *malloc(int n);
void free(void *p);
void abort(void);
int *find(int k);
int *next(void);
void use(int *p);

int f(int *q) {
    int *p;
    int i;
    for (i = 0; i < 1; i++) {
        p = next();
    }
    return p == q;
}

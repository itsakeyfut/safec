void *malloc(int n);
void free(void *p);
void abort(void);
int *find(int k);
int *next(void);
void use(int *p);

int f(int c, int *q) {
    int *p;
    if (c) {
        p = q;
    }
    if (!c) {
        abort();
    }
    use(p);
    return 0;
}

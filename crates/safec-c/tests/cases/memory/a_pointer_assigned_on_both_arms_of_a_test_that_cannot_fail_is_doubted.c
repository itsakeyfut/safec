void *malloc(int n);
void free(void *p);
void abort(void);
int *find(int k);
int *next(void);
void use(int *p);

int f(int n, int *q, int *r) {
    int *p;
    if (n >= 0) {
        p = q;
    } else if (n < 0) {
        p = r;
    }
    use(p);
    return 0;
}

void *malloc(int n);
void free(void *p);
void abort(void);
int *find(int k);
int *next(void);
void use(int *p);

int f(int verbose, int *q) {
    int *p;
    if (verbose) {
        p = q;
    }
    if (verbose) {
        use(p);
    }
    return 0;
}

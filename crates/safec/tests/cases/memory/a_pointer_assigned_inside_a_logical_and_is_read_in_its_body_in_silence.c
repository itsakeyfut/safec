void *malloc(int n);
void free(void *p);
void abort(void);
int *find(int k);
int *next(void);
void use(int *p);

int f(int key, int *q) {
    int *p;
    if (key && (p = find(key)) != 0) {
        return p == q;
    }
    return 0;
}

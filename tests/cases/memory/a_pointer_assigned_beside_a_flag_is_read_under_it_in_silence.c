void *malloc(int n);
void free(void *p);
void abort(void);
int *find(int k);
int *next(void);
void use(int *p);

int f(int c) {
    int *p = malloc(4);
    int *keep;
    int saved = 0;
    if (c) {
        keep = p;
        saved = 1;
    }
    if (saved) {
        free(keep);
    } else {
        free(p);
    }
    return 0;
}

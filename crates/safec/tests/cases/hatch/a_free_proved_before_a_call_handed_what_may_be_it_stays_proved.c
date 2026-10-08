void *malloc(int n);
void free(void *p);
void log_ptr(int *p);

__attribute__((annotate("safec_unchecked")))
int f(int c) {
    int *p = malloc(4);
    int *o = malloc(4);
    if (p == 0) {
        return 0;
    }
    int *r = p;
    if (c) {
        r = o;
    }
    free(p);
    log_ptr(r);
    return *p;
}

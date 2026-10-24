void *malloc(int n);
void free(void *p);
void abort(void);

int f(void) {
    int *p = malloc(4);
    if (p == 0) {
        (abort(), 0);
        free(p);
        free(p);
    }
    return 0;
}

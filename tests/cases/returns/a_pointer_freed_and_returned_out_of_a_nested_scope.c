void *malloc(int n);
void free(void *p);

int *f(void) {
    int *q = malloc(4);
    {
        int *p = q;
        free(p);
        return p;
    }
}

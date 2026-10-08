void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
int f(void) {
    int *r = malloc(4);
    if (r == 0) {
        return 0;
    }
    int **pr = &r;
    int ***ppr = &pr;
    int *s = 0;
    memcpy(&s, *ppr, 8);
    free(r);
    return *s;
}

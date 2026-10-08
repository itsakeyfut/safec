void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);
int f(void) {
    int *r = malloc(4);
    if (r == 0) {
        return 0;
    }
    int *s = 0;
    int **ps = &s;
    int ***pps = &ps;
    memcpy(*pps, &r, 8);
    free(r);
    return *s;
}

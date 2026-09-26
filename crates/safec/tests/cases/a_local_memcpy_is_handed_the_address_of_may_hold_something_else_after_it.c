void *malloc(int n);
void free(void *p);
void *memcpy(void *d, void *s, int n);

int f(void) {
    int *p = malloc(4);
    int *r = p;
    int *z = malloc(4);
    memcpy(&p, &z, 8);
    free(p);
    *r = 1;
    return 0;
}

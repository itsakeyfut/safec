void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(8);
    int *r = &p[1];
    free(p);
    *r = 1;
    return 0;
}

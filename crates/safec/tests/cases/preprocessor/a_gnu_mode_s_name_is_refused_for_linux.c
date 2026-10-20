void *malloc(int n);
void free(void *p);
int f(void) {
    int *p = malloc(4);
    free(p);
#ifdef unix
    free(p);
#endif
    return 0;
}

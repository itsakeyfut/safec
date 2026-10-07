void free(void *p);
int *id(int *p);

int g(void) {
    int x;
    int *r = id(&x);
    free(r);
    return 0;
}

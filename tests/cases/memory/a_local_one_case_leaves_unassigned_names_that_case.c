void *malloc(int n);
void free(void *p);
int f(int n) {
    int *p;
    switch (n) {
    case 0:
        p = malloc(4);
        break;
    case 5:
        break;
    default:
        p = 0;
    }
    free(p);
    return 0;
}

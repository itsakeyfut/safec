void *malloc(int n);
void free(void *p);
int f(int c) {
    int *p = malloc(4);
    switch (c) {
    case 1:
        free(p);
        *p = 1;
        break;
    default:
        break;
    }
    return 0;
}

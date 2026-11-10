void *malloc(int n);
void *returned(void) {
    int *c = malloc(8);
    return c;
}

void *malloc(int n);
void free(void *p);
int main(void) {
    int *x = malloc(4);
    if (x == 0) {
        return 1;
    }
    int *m[1];
    m[0] = x;
    free(x);
    return 0;
}

void *malloc(int n);
void free(void *p);
void release_all(void);

int main(void) {
    int x;
    int r;
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    r = (x = a[0]) + (release_all(), 0);
    free(a);
    return r;
}

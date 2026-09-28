void *malloc(int n);
void *memset(void *s, int c, int n);
int release_all(void);

int main(void) {
    int x;
    int r;
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    a[0] = 1;
    r = (x = a[0]) + (memset(a, 0, 4) != 0) + release_all();
    return r;
}

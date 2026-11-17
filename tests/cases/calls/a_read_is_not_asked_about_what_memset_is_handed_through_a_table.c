void *malloc(int n);
void *memset(void *s, int c, int n);
int release_all(void);

int main(void) {
    int x;
    int r;
    int **tab = malloc(8);
    int *a = malloc(4);
    if (tab == 0) {
        return 0;
    }
    if (a == 0) {
        return 0;
    }
    *tab = a;
    a[0] = 1;
    r = (x = a[0]) + (memset(tab, 0, 8) != 0) + release_all();
    return r;
}

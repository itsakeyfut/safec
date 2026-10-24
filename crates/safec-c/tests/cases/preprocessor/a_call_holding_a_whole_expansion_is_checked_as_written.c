void *malloc(int n);
int release(int *p);
#define G release

int main(void) {
    int *p = malloc(4);
    if (!p) return 0;
    int x;
    int y = (x = *p) + G(p);
    return y;
}

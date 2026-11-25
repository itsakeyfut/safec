void *malloc(int n);
int release(int *p);
#define F(q) (x = *q) + release

int main(void) {
    int *p = malloc(4);
    if (!p) return 0;
    int x;
    int y = F(p)(p);
    return y;
}

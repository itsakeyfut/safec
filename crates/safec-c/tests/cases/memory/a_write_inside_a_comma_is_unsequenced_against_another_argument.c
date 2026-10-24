void h(int v, int w);
int main(void) {
    int x = 1;
    int *q = 0;
    h(*q, (q = &x, 0));
    return 0;
}

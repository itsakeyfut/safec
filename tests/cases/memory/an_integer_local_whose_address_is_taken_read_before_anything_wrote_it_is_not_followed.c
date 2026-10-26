int main(void) {
    int v;
    int *pv = &v;
    int w = *pv;
    return w + v;
}

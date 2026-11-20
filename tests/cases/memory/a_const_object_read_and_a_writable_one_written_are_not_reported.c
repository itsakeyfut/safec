const int g = 1;
int h;
int main(void) {
    const int *c = &g;
    int *p = &h;
    *p = *c;
    return g;
}

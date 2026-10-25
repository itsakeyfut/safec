void g(void *v);

int main(void) {
    char c = 0;
    g(&c);
    return c;
}

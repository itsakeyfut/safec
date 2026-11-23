enum E { A, B = A + 2, C };
int main(void) {
    enum E e = C;
    return e;
}

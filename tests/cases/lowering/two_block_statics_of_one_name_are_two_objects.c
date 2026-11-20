int main(void) {
    {
        static int a = 2;
    }
    {
        static int a = 3;
        return a;
    }
}
